//! Model downloads and the HuggingFace hub cache: verified streaming
//! download with resume, cache discovery/migration, blob/snapshot
//! management.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::catalog::{ModelSpec, LLM_MODELS, STT_MODELS};

/// User-Agent for all outbound HTTP (downloads, external endpoints).
pub(crate) const USER_AGENT: &str = concat!("brainstory-desktop/", env!("CARGO_PKG_VERSION"));

pub fn model_url(spec: &ModelSpec, endpoint: &str) -> String {
	format!("{}/{}/resolve/main/{}", endpoint, spec.repo, spec.filename)
}

/// Every hub-cache directory a model file could already live in, in
/// huggingface_hub precedence order: `HF_HUB_CACHE`, then the legacy
/// `HUGGINGFACE_HUB_CACHE`, then `HF_HOME/hub`, then
/// `$XDG_CACHE_HOME/huggingface/hub` (the Python client honors XDG; the
/// Rust hf-hub crate does not - probing costs nothing), then the
/// platform-independent default `~/.cache/huggingface/hub` (HF uses
/// ~/.cache even on macOS/Windows, never the platform cache dirs).
/// Discovery is read-only, so extra candidates are harmless.
pub fn hf_hub_cache_candidates() -> Vec<PathBuf> {
	let mut candidates: Vec<PathBuf> = Vec::new();
	let mut push = |p: Option<PathBuf>| {
		if let Some(p) = p.filter(|p| !p.as_os_str().is_empty()) {
			if !candidates.contains(&p) {
				candidates.push(p);
			}
		}
	};
	for var in ["HF_HUB_CACHE", "HUGGINGFACE_HUB_CACHE"] {
		push(std::env::var(var).ok().map(|v| PathBuf::from(v.trim())));
	}
	push(
		std::env::var("HF_HOME")
			.ok()
			.map(|v| PathBuf::from(v.trim()).join("hub")),
	);
	push(
		std::env::var("XDG_CACHE_HOME")
			.ok()
			.map(|v| PathBuf::from(v.trim()).join("huggingface").join("hub")),
	);
	push(dirs::home_dir().map(|home| home.join(".cache").join("huggingface").join("hub")));
	candidates
}

/// Locate `spec`'s file inside a HuggingFace hub cache directory
/// (`models--<org>--<repo>/snapshots/<rev>/<filename>`). Symlinked
/// snapshot files (the normal layout) resolve through `is_file`.
/// Returns the newest snapshot that contains the file.
pub fn hf_cache_model_path(cache_dir: &Path, spec: &ModelSpec) -> Option<PathBuf> {
	let repo_dir = cache_dir.join(format!("models--{}", spec.repo.replace('/', "--")));
	let snapshots = repo_dir.join("snapshots");
	let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
	for entry in std::fs::read_dir(&snapshots).ok()?.flatten() {
		let candidate = entry.path().join(spec.filename);
		if !candidate.is_file() {
			continue;
		}
		let modified = entry
			.metadata()
			.ok()
			.and_then(|m| m.modified().ok())
			.unwrap_or(std::time::SystemTime::UNIX_EPOCH);
		if best.as_ref().is_none_or(|(t, _)| modified > *t) {
			best = Some((modified, candidate));
		}
	}
	best.map(|(_, path)| path)
}

/// The one cache directory Brainstory writes into: the first
/// environment-configured candidate, else the default. Discovery reads
/// every candidate; writes need exactly one target.
pub fn primary_hub_cache() -> PathBuf {
	hf_hub_cache_candidates()
		.into_iter()
		.next()
		.unwrap_or_else(|| {
			dirs::home_dir()
				.unwrap_or_else(|| PathBuf::from("."))
				.join(".cache")
				.join("huggingface")
				.join("hub")
		})
}

fn hf_repo_dir(cache: &Path, spec: &ModelSpec) -> PathBuf {
	cache.join(format!("models--{}", spec.repo.replace('/', "--")))
}

/// The content-addressed blob for this model: `blobs/<pinned sha256>`,
/// the same name huggingface tooling uses for LFS files, so a blob we
/// download or migrate is deduped against theirs automatically.
pub fn hf_blob_path(cache: &Path, spec: &ModelSpec) -> PathBuf {
	hf_repo_dir(cache, spec).join("blobs").join(spec.sha256)
}

/// Link a blob into a snapshot under `snapshots/<sha>/<filename>`,
/// trying a relative symlink (the standard layout) first, then a
/// hardlink (Windows without symlink privileges, or a symlink that
/// doesn't resolve), then a copy. Idempotent; a dead link is replaced.
pub fn materialize_snapshot(cache: &Path, spec: &ModelSpec) -> Result<(), String> {
	let snapshot_dir = hf_repo_dir(cache, spec).join("snapshots").join(spec.sha256);
	std::fs::create_dir_all(&snapshot_dir).map_err(|e| e.to_string())?;
	let link = snapshot_dir.join(spec.filename);
	if link.symlink_metadata().is_ok() {
		if link.is_file() {
			return Ok(());
		}
		// a link that doesn't resolve (e.g. an older build's Windows link
		// with `/` separators): replace it rather than keep a dead entry
		std::fs::remove_file(&link).map_err(|e| e.to_string())?;
	}
	let blob = hf_blob_path(cache, spec);
	if !blob.is_file() {
		return Err(format!("blob missing for {}", spec.id));
	}
	// built from components so Windows gets `..\..\blobs\<sha>`: it
	// creates a symlink whose relative target uses `/` without complaint,
	// but never resolves it
	let rel: PathBuf = ["..", "..", "blobs", spec.sha256].iter().collect();
	#[cfg(target_family = "unix")]
	{
		std::os::unix::fs::symlink(&rel, &link).map_err(|e| e.to_string())?;
	}
	#[cfg(target_os = "windows")]
	{
		use std::os::windows::fs as win_fs;
		let linked = win_fs::symlink_file(&rel, &link).is_ok() && link.is_file();
		if !linked {
			let _ = std::fs::remove_file(&link);
			std::fs::hard_link(&blob, &link)
				.or_else(|_| std::fs::copy(&blob, &link).map(|_| ()))
				.map_err(|e| e.to_string())?;
		}
	}
	Ok(())
}

/// Streaming sha256 of a file, lowercase hex.
fn sha256_of_file(path: &Path) -> Option<String> {
	use sha2::{Digest, Sha256};
	let mut file = std::fs::File::open(path).ok()?;
	let mut hasher = Sha256::new();
	let mut buf = vec![0u8; 1024 * 1024];
	use std::io::Read;
	loop {
		let n = file.read(&mut buf).ok()?;
		if n == 0 {
			break;
		}
		hasher.update(&buf[..n]);
	}
	Some(
		hasher
			.finalize()
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect(),
	)
}

/// One-time migration: move legacy app-dir model files into the hub
/// cache so one copy serves Brainstory and every other HF tool. The
/// source is hash-verified first - a corrupt or foreign file must never
/// be copied into a content-addressed store under a sha it doesn't
/// have. An existing blob counts as "already cached" only once its
/// bytes verify against the pin, and the app copy is dropped only
/// after a verified blob is published under a snapshot the resolver
/// can read, so every failure leaves a usable copy behind.
pub fn migrate_legacy_models(models_dir: &Path, cache: &Path) {
	for spec in LLM_MODELS.iter().chain(STT_MODELS.iter()) {
		migrate_one(models_dir, cache, spec);
	}
}

/// Migrate a single spec's app-dir file into the cache (separate so
/// tests can drive it with fixture specs instead of the catalog pins).
///
/// Ordering contract (verify-then-delete): the app-dir copy is the
/// last known-good source, so it is deleted only after the destination
/// is independently usable - a blob whose streamed sha256 matches the
/// pin AND a snapshot entry that resolves (the resolver in state.rs
/// reads `snapshots/<rev>/<filename>`, never a bare blob). Same-volume
/// and cross-volume moves share the temp-copy/verify/publish ordering:
/// a same-volume rename would move the only good copy to a staging
/// name nothing resolves (and the startup sweep reclaims a
/// full-length `.part`), while a copy leaves the app copy discoverable
/// until the published blob plus snapshot replace it. Every failure
/// path keeps either the untouched original or a verified copy, and
/// the whole migration is idempotent, so a retry after an
/// interruption is always safe.
fn migrate_one(models_dir: &Path, cache: &Path, spec: &ModelSpec) {
	let app_file = models_dir.join(spec.filename);
	let blob = hf_blob_path(cache, spec);

	// Fast path: nothing to migrate and the cache is already in its
	// final state. This runs on EVERY launch for EVERY catalog model,
	// so it must stay cheap: re-verifying the multi-GB blobs below on
	// every startup cost tens of seconds before the engines could
	// load, leaving the UI reporting "no model set up" for most of a
	// minute. The hash-verified paths only run when there is actual
	// migration or healing work to decide.
	if !app_file.is_file() && hf_cache_model_path(cache, spec).is_some() {
		return;
	}

	// An existing hash-named blob is trusted only after its BYTES
	// verify (size short-circuit + streamed sha256) against the pin:
	// the name alone proves nothing, and a corrupt blob must never get
	// the good app copy deleted. A blob that cannot be read is
	// uncertainty, not validity - keep everything, retry next launch.
	let blob_meta = std::fs::metadata(&blob);
	if let Err(e) = &blob_meta {
		if e.kind() != std::io::ErrorKind::NotFound {
			log::warn!(
				"cannot inspect the cache blob of {}: {e}; leaving the app copy in place",
				spec.id
			);
			return;
		}
	}
	let blob_exists = blob_meta.is_ok();
	let mut blob_verified = false;
	if let Ok(meta) = &blob_meta {
		if meta.is_file() {
			match snapshot_file_is_pinned(&blob, meta, spec) {
				Some(true) => blob_verified = true,
				// present but not the pinned bytes: the publish below
				// repairs the sha-named slot with the real content
				Some(false) => {}
				None => {
					log::warn!(
						"cannot verify the cache blob of {}: leaving the app copy in place",
						spec.id
					);
					return;
				}
			}
		}
	}

	if !app_file.is_file() {
		// Nothing to migrate. Still make an already-verified blob
		// discoverable: the pre-fix ordering renamed the app copy into
		// the blob before publishing the snapshot, so an interrupted
		// run could leave an undiscoverable blob and no app copy.
		if blob_verified {
			if let Err(e) = materialize_snapshot(cache, spec) {
				log::warn!("could not create the cache snapshot for {}: {e}", spec.id);
			}
		} else if blob_exists {
			log::warn!(
				"the cached copy of {} does not match the pinned content; re-download the model to repair it",
				spec.id
			);
		}
		return;
	}

	if !blob_verified {
		// The app copy is the migration source: it must BE the pinned
		// content before anything is published under the pinned name.
		let app_meta = match std::fs::metadata(&app_file) {
			Ok(meta) => meta,
			Err(e) => {
				log::warn!(
					"cannot inspect the app copy of {}: {e}; leaving it in place",
					spec.id
				);
				return;
			}
		};
		match snapshot_file_is_pinned(&app_file, &app_meta, spec) {
			Some(true) => {}
			Some(false) => {
				log::warn!(
					"leaving {} in the app models dir: its content does not match the pinned hash",
					spec.id
				);
				return;
			}
			None => {
				log::warn!(
					"leaving {} in the app models dir: its content could not be verified",
					spec.id
				);
				return;
			}
		}
		if let Some(parent) = blob.parent() {
			if let Err(e) = std::fs::create_dir_all(parent) {
				log::warn!("could not create the cache blobs dir: {e}");
				return;
			}
		}
		// temp-copy/verify/publish on every volume: a partial or bad
		// copy never lands under the final name, and the app copy
		// stays put until the blob is published
		let tmp = part_path(&blob);
		if let Err(e) = std::fs::copy(&app_file, &tmp) {
			log::warn!("could not copy {} into the cache: {e}", spec.id);
			let _ = std::fs::remove_file(&tmp);
			return;
		}
		// verify the staged bytes before publishing them: a faulty
		// disk must not put unverified content under the pinned name
		let staged = std::fs::metadata(&tmp)
			.ok()
			.and_then(|meta| snapshot_file_is_pinned(&tmp, &meta, spec));
		if staged != Some(true) {
			log::warn!(
				"the staged copy of {} did not verify ({staged:?}); keeping the app copy",
				spec.id
			);
			let _ = std::fs::remove_file(&tmp);
			return;
		}
		// atomic publish. A unix rename replaces an existing entry;
		// where it cannot (Windows over an existing file, or a non-file
		// squatting on the name and refusing removal), the failure
		// keeps the app copy - and the removal only ever targets a
		// blob this run already proved unpinned, never verified data.
		if let Err(e) = std::fs::rename(&tmp, &blob).or_else(|first| {
			std::fs::remove_file(&blob)
				.and_then(|_| std::fs::rename(&tmp, &blob))
				.map_err(|_| first)
		}) {
			log::warn!("could not migrate {} into the cache: {e}", spec.id);
			let _ = std::fs::remove_file(&tmp);
			return;
		}
	}

	// The blob is verified; publish the snapshot BEFORE dropping the
	// app copy - a bare blob is not discoverable, so the snapshot is
	// what completes the migration.
	if let Err(e) = materialize_snapshot(cache, spec) {
		log::warn!(
			"could not create the cache snapshot for {}: {e}; keeping the app copy",
			spec.id
		);
		return;
	}
	if hf_cache_model_path(cache, spec).is_none() {
		log::warn!(
			"the cache snapshot of {} did not resolve; keeping the app copy",
			spec.id
		);
		return;
	}
	// A verified, discoverable destination exists: the app copy is now
	// redundant. A failed removal is harmless - the next launch
	// retries it through the verified-blob path above.
	if let Err(e) = std::fs::remove_file(&app_file) {
		log::warn!(
			"migrated {} but could not remove the redundant app copy: {e}",
			spec.id
		);
		return;
	}
	log::info!("migrated {} into the hub cache", spec.id);
}

/// Whether some snapshot entry still links to `blobs/<sha>`, from a
/// scan that actually completed. An unreadable directory or entry is
/// returned as `Err` - uncertainty, never absence: the caller must
/// retain the blob rather than prune it on a failed scan. A broken
/// symlink references nothing. A non-symlink entry (hardlink or
/// copied fallback, e.g. on Windows) hides its target, so it counts
/// as referencing - never prune what might be in use.
fn blob_referenced(snapshots_dir: &Path, sha: &str) -> Result<bool, String> {
	let revs = std::fs::read_dir(snapshots_dir)
		.map_err(|e| format!("cannot scan {}: {e}", snapshots_dir.display()))?;
	for rev in revs {
		let rev = rev.map_err(|e| format!("cannot scan {}: {e}", snapshots_dir.display()))?;
		let rev_dir = rev.path();
		if !rev_dir.is_dir() {
			continue;
		}
		let entries = std::fs::read_dir(&rev_dir)
			.map_err(|e| format!("cannot scan {}: {e}", rev_dir.display()))?;
		for entry in entries {
			let entry = entry.map_err(|e| format!("cannot scan {}: {e}", rev_dir.display()))?;
			let file_type = entry
				.file_type()
				.map_err(|e| format!("cannot inspect {}: {e}", entry.path().display()))?;
			let path = entry.path();
			if file_type.is_symlink() {
				// a link that does not resolve references nothing
				if !path.is_file() {
					continue;
				}
				match std::fs::read_link(&path) {
					Ok(target) => {
						if target.file_name().map(|n| n == sha).unwrap_or(false) {
							return Ok(true);
						}
					}
					Err(_) => return Ok(true), // unreadable link: conservatively in use
				}
			} else if !file_type.is_dir() {
				// not a symlink: a hardlink or copied fallback entry
				// hides its target - conservatively in use
				return Ok(true);
			}
		}
	}
	Ok(false)
}

/// Whether a regular file (a snapshot entry's hardlink/copy fallback,
/// e.g. on Windows, a hash-named blob, or an app-dir model copy)
/// holds the pinned content: the catalog size short-circuits before
/// any hashing, then the streamed sha256 must match the pin. `None` =
/// could not verify (a read error): the caller must retain the file.
fn snapshot_file_is_pinned(
	target: &Path,
	meta: &std::fs::Metadata,
	spec: &ModelSpec,
) -> Option<bool> {
	if spec.size_bytes > 0 && meta.len() != spec.size_bytes {
		return Some(false);
	}
	sha256_of_file(target).map(|hash| hash.eq_ignore_ascii_case(spec.sha256))
}

/// Remove this model's cache entry from its own catalog repo, kept
/// for the pinned content only - never a same-named file that holds
/// something else. A snapshot link goes when its target is
/// `blobs/<pinned sha>`; a regular file only after its size matches
/// the catalog and its streamed sha256 matches the pin; anything
/// unidentifiable (read errors, unresolvable links) is retained and
/// logged, so a delete cannot break another tool's revision (worst
/// case, a tool whose snapshot was intentionally kept re-downloads a
/// blob we removed as unreferenced).
pub fn remove_cached_model(cache: &Path, spec: &ModelSpec) -> Result<bool, String> {
	let repo_dir = hf_repo_dir(cache, spec);
	let snapshots = repo_dir.join("snapshots");
	if !snapshots.is_dir() {
		return Ok(false);
	}
	let mut removed = false;
	let revs = match std::fs::read_dir(&snapshots) {
		Ok(revs) => revs,
		Err(e) => {
			log::warn!(
				"could not list {}: {e}; no snapshot entry was removed",
				snapshots.display()
			);
			return Ok(false);
		}
	};
	for rev in revs {
		let rev = match rev {
			Ok(rev) => rev,
			Err(e) => {
				log::warn!("could not read a snapshot revision entry: {e}");
				continue;
			}
		};
		let target = rev.path().join(spec.filename);
		let meta = match target.symlink_metadata() {
			Ok(meta) => meta,
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
			Err(e) => {
				log::warn!("skipping {}: cannot inspect it: {e}", target.display());
				continue;
			}
		};
		let file_type = meta.file_type();
		if file_type.is_symlink() {
			let targets_pin = std::fs::read_link(&target)
				.ok()
				.and_then(|t| t.file_name().map(|n| n == spec.sha256));
			match targets_pin {
				Some(true) => {
					std::fs::remove_file(&target).map_err(|e| e.to_string())?;
					removed = true;
				}
				Some(false) => {} // a different revision's link to other content
				None => log::warn!("skipping {}: cannot resolve the link", target.display()),
			}
		} else if file_type.is_file() {
			match snapshot_file_is_pinned(&target, &meta, spec) {
				Some(true) => {
					std::fs::remove_file(&target).map_err(|e| e.to_string())?;
					removed = true;
				}
				Some(false) => {} // same name, different content: not ours
				None => log::warn!("skipping {}: cannot verify its content", target.display()),
			}
		} else {
			log::warn!(
				"skipping {}: not a file the app published",
				target.display()
			);
		}
	}
	let blob = hf_blob_path(cache, spec);
	if blob.is_file() {
		match blob_referenced(&snapshots, spec.sha256) {
			Ok(false) => {
				std::fs::remove_file(&blob).map_err(|e| e.to_string())?;
				removed = true;
			}
			// a completed scan found a live reference, or the scan
			// itself failed: either way the blob stays
			Ok(true) => {}
			Err(e) => log::warn!(
				"retaining the blob of {}: the reference scan did not complete: {e}",
				spec.id
			),
		}
	}
	Ok(removed)
}

/// How long a resumable `.part` is kept around waiting for its download
/// to be retried before the startup sweep reclaims the disk space.
const PART_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// Startup cleanup of `.part` staging files. Downloads resume from a
/// `.part` across restarts, so only files that can never be resumed go:
/// - in the legacy app models dir, every `.part` (downloads no longer
///   stage there; leftovers are interrupted migration copies, which
///   restart from scratch anyway);
/// - in the blobs dir of a catalog repo, a `.part` that is not
///   `<pinned sha>.part` of a catalog model, is not shorter than that
///   model, or has not been touched for [`PART_MAX_AGE`].
///
/// Repos that are not in the catalog belong to other HuggingFace tools
/// and are never touched.
pub fn sweep_stale_part_files(models_dir: &Path, cache_dir: &Path) {
	let is_part = |path: &Path| path.extension().map(|e| e == "part").unwrap_or(false);
	let remove = |path: &Path| {
		log::warn!("removing leftover partial download {}", path.display());
		if let Err(e) = std::fs::remove_file(path) {
			log::warn!("could not remove {}: {e}", path.display());
		}
	};
	for entry in std::fs::read_dir(models_dir)
		.into_iter()
		.flatten()
		.flatten()
	{
		if is_part(&entry.path()) {
			remove(&entry.path());
		}
	}

	let specs: Vec<&ModelSpec> = LLM_MODELS.iter().chain(STT_MODELS.iter()).collect();
	let mut repos: Vec<&str> = specs.iter().map(|s| s.repo).collect();
	repos.sort_unstable();
	repos.dedup();
	for repo in repos {
		let blobs = cache_dir
			.join(format!("models--{}", repo.replace('/', "--")))
			.join("blobs");
		for entry in std::fs::read_dir(&blobs).into_iter().flatten().flatten() {
			let path = entry.path();
			if !is_part(&path) {
				continue;
			}
			let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
			let Some(spec) = specs.iter().find(|s| s.repo == repo && s.sha256 == stem) else {
				remove(&path);
				continue;
			};
			let meta = entry.metadata().ok();
			let resumable = meta.as_ref().is_some_and(|m| m.len() < spec.size_bytes);
			let fresh = meta
				.and_then(|m| m.modified().ok())
				.and_then(|t| t.elapsed().ok())
				.is_some_and(|age| age < PART_MAX_AGE);
			if resumable && fresh {
				log::info!("keeping partial download of {} for a resume", spec.id);
			} else {
				remove(&path);
			}
		}
	}
}

/// The `.part` staging path for a download destination: `<file>.part`
/// appended to the full name (with_extension would collapse `x.bin` and
/// `x.gguf` to the same `x.part`).
fn part_path(dest: &Path) -> PathBuf {
	let mut name = dest.as_os_str().to_os_string();
	name.push(".part");
	PathBuf::from(name)
}

/// Parse a `Content-Range: bytes <start>-<end>/<total|*>` header into
/// `(start, end, total)`, with `total` None for `*` (unknown). None
/// when the header is missing or malformed: wrong unit, unparsable
/// numbers, `end < start`, or a declared total that cannot cover `end`.
fn parse_content_range(header: Option<&str>) -> Option<(u64, u64, Option<u64>)> {
	let header = header?.trim();
	let (unit, spec) = header.split_once(' ')?;
	if !unit.trim().eq_ignore_ascii_case("bytes") {
		return None;
	}
	let (range, total) = spec.trim().split_once('/')?;
	let total = match total.trim() {
		"*" => None,
		digits => Some(digits.parse::<u64>().ok()?),
	};
	let (start, end) = range.trim().split_once('-')?;
	let start = start.trim().parse::<u64>().ok()?;
	let end = end.trim().parse::<u64>().ok()?;
	if end < start || total.is_some_and(|t| t <= end) {
		return None;
	}
	Some((start, end, total))
}

/// Why a streaming download stopped, and whether the staged bytes are
/// still a valid prefix worth resuming from.
enum StreamFailure {
	/// Transient (stall, dropped connection, early end of stream): the
	/// `.part` holds a correct prefix and is kept for a Range resume -
	/// even across an app restart.
	Resumable(String),
	/// Cancelled by the user, corrupt, oversized or unwritable: the
	/// staged bytes are worthless (or unwanted) and are removed.
	Discard(String),
}

/// The running total after one more chunk arrives, or None when the
/// counter would wrap (checked arithmetic: a server lying with huge
/// chunked sizes cannot overflow the byte count into a bogus small
/// total that slips past the size checks).
fn checked_next_total(downloaded: u64, chunk_len: u64) -> Option<u64> {
	downloaded.checked_add(chunk_len)
}

/// Deadline configuration for the download transport. Production runs
/// [`Timeouts::PRODUCTION`]; tests inject tiny values so deadline and
/// cancellation paths run in milliseconds instead of 30/60 seconds.
///
/// There is deliberately no overall whole-download deadline: multi-GB
/// models on slow links are the supported slow-transfer policy - as
/// long as bytes keep arriving (each within `stall`), a transfer may
/// legitimately take hours.
#[derive(Clone, Copy)]
struct Timeouts {
	/// Overall bound on awaiting response headers once the request is
	/// sent: a connected server that never answers cannot pin the
	/// download (and its cancel path) forever. Firing is transient -
	/// the staged prefix survives it for a resume.
	header: std::time::Duration,
	/// A body read is stalled when not a single byte arrives for this
	/// long; the stall timer only resets on data, so cancel polling
	/// never extends it.
	stall: std::time::Duration,
	/// How often the cancel flag is polled while awaiting headers or
	/// body chunks, so a cancel lands within a fraction of a second
	/// instead of waiting out `header`/`stall`.
	cancel_poll: std::time::Duration,
}

impl Timeouts {
	const PRODUCTION: Timeouts = Timeouts {
		header: std::time::Duration::from_secs(30),
		stall: std::time::Duration::from_secs(60),
		cancel_poll: std::time::Duration::from_millis(150),
	};
}

/// Stream a model file to disk, reporting progress through `on_progress`
/// (percentage 0-100). Verifies the download completed fully and matches
/// the pinned sha256 before moving it into place. A transient failure
/// keeps the `.part` file so the next attempt resumes it; cancellation
/// and integrity/size failures remove it. Every wait - headers, body
/// chunks - is bounded and cancellation-aware (see [`Timeouts`]); any
/// byte that would outgrow `expected_size` is rejected before it is
/// written.
pub async fn download_model_file(
	url: &str,
	dest: &Path,
	expected_size: u64,
	expected_sha256: &str,
	hf_token: &str,
	cancel: &AtomicBool,
	on_progress: &mut (impl FnMut(f64) + Send),
) -> Result<(), String> {
	download_with_timeouts(
		url,
		dest,
		expected_size,
		expected_sha256,
		hf_token,
		cancel,
		on_progress,
		Timeouts::PRODUCTION,
	)
	.await
}

#[allow(clippy::too_many_arguments)]
async fn download_with_timeouts(
	url: &str,
	dest: &Path,
	expected_size: u64,
	expected_sha256: &str,
	hf_token: &str,
	cancel: &AtomicBool,
	on_progress: &mut (impl FnMut(f64) + Send),
	timeouts: Timeouts,
) -> Result<(), String> {
	use sha2::{Digest, Sha256};

	let tmp = part_path(dest);

	let client = reqwest::Client::builder()
		.connect_timeout(std::time::Duration::from_secs(15))
		.build()
		.map_err(|e| e.to_string())?;

	// Resume support: a leftover .part from a quit mid-download can be
	// continued with a Range request instead of restarting multi-GB from
	// zero. The existing bytes are hashed while streaming them from disk,
	// so the final sha256 check still covers the whole file.
	let mut hasher = (!expected_sha256.is_empty()).then(Sha256::new);
	let mut downloaded: u64 = 0;
	let mut resume_from: u64 = 0;
	if let Ok(meta) = std::fs::metadata(&tmp) {
		resume_from = meta.len();
		// Only resume when the prefix can still matter: a .part larger
		// than the expected file is junk from a different state.
		if expected_size > 0 && resume_from >= expected_size {
			tokio::fs::remove_file(&tmp)
				.await
				.map_err(|e| e.to_string())?;
			resume_from = 0;
		}
	} else if tmp.exists() {
		// exists but unreadable metadata: start over
		tokio::fs::remove_file(&tmp)
			.await
			.map_err(|e| e.to_string())?;
	}

	let mut request = client.get(url).header("User-Agent", USER_AGENT);
	if resume_from > 0 {
		request = request.header("Range", format!("bytes={resume_from}-"));
	}
	if !hf_token.is_empty() {
		request = request.bearer_auth(hf_token);
	}
	// Bounded, cancellation-aware header wait: a connected server that
	// withholds its response cannot pin the download forever. The send
	// future is raced with a short cancel tick; the overall deadline
	// is re-checked on every tick. A deadline firing is a transient
	// failure (the staged prefix survives it, like a dropped
	// connection), while an explicit cancel discards it like every
	// other cancel.
	let header_deadline = tokio::time::Instant::now() + timeouts.header;
	let send = request.send();
	tokio::pin!(send);
	let response = loop {
		if cancel.load(Ordering::Relaxed) {
			let _ = tokio::fs::remove_file(&tmp).await;
			return Err("download cancelled".into());
		}
		let now = tokio::time::Instant::now();
		if now >= header_deadline {
			return Err(format!(
				"download timed out waiting for the server's response ({}s) - please retry",
				timeouts.header.as_secs()
			));
		}
		let wait = timeouts.cancel_poll.min(header_deadline - now);
		match tokio::time::timeout(wait, send.as_mut()).await {
			Ok(Ok(response)) => break response,
			Ok(Err(e)) => return Err(format!("download request failed: {e}")),
			Err(_) => continue, // tick elapsed: re-check cancel and deadline
		}
	};
	if response.status() == reqwest::StatusCode::UNAUTHORIZED
		|| response.status() == reqwest::StatusCode::FORBIDDEN
	{
		return Err(format!(
			"download not authorized ({}) - check the HuggingFace access token in AI Models settings",
			response.status()
		));
	}
	if !response.status().is_success() {
		return Err(format!("download failed with status {}", response.status()));
	}

	// A server that ignores Range answers 200 with the full body; the
	// stale .part cannot be stitched onto it, so restart from zero.
	let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
	if resume_from > 0 && !resumed {
		tokio::fs::remove_file(&tmp)
			.await
			.map_err(|e| e.to_string())?;
		resume_from = 0;
		hasher = (!expected_sha256.is_empty()).then(Sha256::new);
	}

	// Validate a 206 before a single byte is appended: the Content-Range
	// start has to equal the resume offset (or the stitched file would
	// be corrupt) and a declared total has to agree with the catalog.
	// A 206 nobody asked for (no Range was sent) is a malformed
	// response. These are bounded failures: the body is never streamed
	// and the staged bytes are discarded like any other integrity
	// failure, never appended to.
	let mut range_total: Option<u64> = None;
	if resumed {
		let header = response
			.headers()
			.get(reqwest::header::CONTENT_RANGE)
			.and_then(|value| value.to_str().ok());
		let rejection = match parse_content_range(header) {
			_ if resume_from == 0 => Some(
				"unexpected 206 Partial Content without a range request - please retry".into(),
			),
			Some((start, _, _)) if start != resume_from => Some(format!(
				"resume position mismatch (server sent bytes from {start}, expected the staged {resume_from}-byte prefix) - please retry"
			)),
			Some((_, _, total)) => {
				range_total = total;
				None
			}
			None => Some(
				"malformed Content-Range header on a 206 response - please retry".into(),
			),
		};
		if let Some(err) = rejection {
			let _ = tokio::fs::remove_file(&tmp).await;
			return Err(err);
		}
	}

	// Hash the resumed prefix from disk so the integrity check still
	// covers the complete file, and pre-seed the byte counter.
	// Cancellation is checked between 1 MiB buffered reads - one read
	// of a local staging file completes in far under a second, so a
	// cancel lands promptly without racing the read future itself
	// (dropping an in-flight tokio::fs read can discard the bytes it
	// read while the file offset still advances, corrupting the hash).
	if resumed {
		if let Some(h) = hasher.as_mut() {
			let mut file = tokio::fs::File::open(&tmp)
				.await
				.map_err(|e| e.to_string())?;
			use tokio::io::AsyncReadExt;
			let mut buf = vec![0u8; 1024 * 1024];
			loop {
				if cancel.load(Ordering::Relaxed) {
					drop(file);
					let _ = tokio::fs::remove_file(&tmp).await;
					return Err("download cancelled".into());
				}
				let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
				if n == 0 {
					break;
				}
				h.update(&buf[..n]);
			}
		}
		downloaded = resume_from;
		log::info!("resuming download at {resume_from} of {expected_size} bytes");
	} else if resume_from == 0 && tmp.exists() {
		// fresh download: the staging file must be empty/new
		tokio::fs::remove_file(&tmp)
			.await
			.map_err(|e| e.to_string())?;
	}

	// The whole-file size this response actually pins down: a 206
	// declares it in its Content-Range total (or as resume offset plus
	// the partial Content-Length), a 200 in its Content-Length. A
	// chunked body may declare none - the total is then genuinely
	// unknown and only the catalog size bounds the finished file; it
	// must not be invented (treating it as the prefix length is what
	// broke chunked resumes).
	let advertised: Option<u64> = if resumed {
		range_total.or_else(|| response.content_length().map(|len| resume_from + len))
	} else {
		response.content_length()
	};
	// Fail fast when the advertised length already contradicts the spec:
	// streaming multi-GB only to reject it at the end wastes the transfer.
	if let Some(known) = advertised {
		if known > 0 && expected_size > 0 && known != expected_size {
			if resumed {
				// the staged prefix belongs to whatever the server is
				// serving, not to the pinned file: not resumable
				let _ = tokio::fs::remove_file(&tmp).await;
			}
			return Err(format!(
				"download size mismatch (server says {known} bytes, expected {expected_size}) - please retry"
			));
		}
	}
	let total = advertised.unwrap_or(0);
	use futures_util::StreamExt;
	let mut stream = response.bytes_stream();
	let mut file = if resumed {
		tokio::fs::OpenOptions::new()
			.append(true)
			.open(&tmp)
			.await
			.map_err(|e| e.to_string())?
	} else {
		tokio::fs::File::create(&tmp)
			.await
			.map_err(|e| e.to_string())?
	};
	use tokio::io::AsyncWriteExt;

	// A transient failure keeps the staged prefix for a Range resume;
	// everything else removes it, so a retry never stitches onto junk.
	let outcome = async {
		use StreamFailure::{Discard, Resumable};
		// `downloaded` comes from the outer scope: it is pre-seeded with
		// the resumed prefix so totals and progress include it.
		let mut last_report: u64 = downloaded;
		// A read is stalled when no byte arrives for the stall timeout.
		// The timer only resets on data, so the short cancel ticks never
		// extend it; cancellation is raced with the same tick so a
		// cancel lands in well under a second instead of waiting out
		// the stall (StreamExt::next is cancel-safe - no chunk is lost
		// when the tick wins the race).
		let stall = tokio::time::sleep(timeouts.stall);
		tokio::pin!(stall);
		loop {
			if cancel.load(Ordering::Relaxed) {
				return Err(Discard("download cancelled".into()));
			}
			let chunk = tokio::select! {
				_ = &mut stall => {
					return Err(Resumable(format!(
						"download stalled (no data for {}s) - please retry",
						timeouts.stall.as_secs()
					)))
				}
				_ = tokio::time::sleep(timeouts.cancel_poll) => continue,
				chunk = stream.next() => chunk,
			};
			stall.as_mut()
				.reset(tokio::time::Instant::now() + timeouts.stall);
			let chunk = match chunk {
				Some(Ok(c)) => c,
				Some(Err(e)) => return Err(Resumable(format!("download interrupted: {e}"))),
				None => break,
			};
			// The catalog size caps the staging file before each write:
			// the running total advances with checked arithmetic (a
			// lying chunked stream cannot overflow it), and any byte
			// that would take the file past `expected_size` fails and
			// discards like an integrity failure - an endless response
			// can never grow the staging file past the pinned size.
			// expected_size == 0 (no catalog bound) is only reachable
			// from local/test callers - the production call site always
			// passes a catalog spec.size_bytes > 0 - so that mode stays
			// uncapped rather than inventing a limit no shipped model
			// needs.
			let next = match checked_next_total(downloaded, chunk.len() as u64) {
				Some(next) => next,
				None => {
					return Err(Discard(
						"download byte counter overflowed - please retry".into(),
					))
				}
			};
			if expected_size > 0 && next > expected_size {
				return Err(Discard(format!(
					"download exceeded the expected size (stream passed {expected_size} bytes) - the extra data was not saved, please retry"
				)));
			}
			if let Some(hasher) = hasher.as_mut() {
				hasher.update(&chunk);
			}
			file.write_all(&chunk)
				.await
				.map_err(|e| Discard(e.to_string()))?;
			downloaded = next;
			if downloaded - last_report > 2_000_000 || downloaded == total {
				last_report = downloaded;
				let pct = if total > 0 {
					(downloaded as f64 / total as f64) * 100.0
				} else {
					// Content-length unknown (chunked transfer): report the
					// indeterminate sentinel; the UI shows a busy bar.
					-1.0
				};
				on_progress(pct);
			}
		}
		file.flush().await.map_err(|e| Discard(e.to_string()))?;
		// The stream can end "cleanly" mid-body; only a full-length file is
		// a valid model, anything else fails to load with cryptic errors.
		// A short file is a resumable prefix; a long one is junk.
		let short_or_junk = |message: String, limit: u64| {
			if downloaded < limit {
				Resumable(message)
			} else {
				Discard(message)
			}
		};
		if total > 0 && downloaded != total {
			return Err(short_or_junk(
				format!("download incomplete (got {downloaded} of {total} bytes) - please retry"),
				total,
			));
		}
		if expected_size > 0 && downloaded != expected_size {
			return Err(short_or_junk(
				format!(
					"download size mismatch (got {downloaded} bytes, expected {expected_size}) - please retry"
				),
				expected_size,
			));
		}
		if let Some(hasher) = hasher.take() {
			let actual: String = hasher
				.finalize()
				.iter()
				.map(|b| format!("{b:02x}"))
				.collect();
			if !actual.eq_ignore_ascii_case(expected_sha256) {
				return Err(Discard(format!(
					"download failed its integrity check (sha256 {actual}) - the file was corrupted in transit or changed upstream; please retry"
				)));
			}
		}
		Ok(())
	}
	.await;

	match outcome {
		Ok(()) => {
			drop(file);
			tokio::fs::rename(&tmp, dest)
				.await
				.map_err(|e| e.to_string())?;
			Ok(())
		}
		Err(StreamFailure::Resumable(e)) => {
			// tokio finishes file writes in the background: flush so the
			// kept prefix is really on disk before anyone resumes from it
			if let Err(flush_err) = file.flush().await {
				log::warn!("could not flush the partial download: {flush_err}");
			}
			drop(file);
			log::info!(
				"keeping {} bytes of {} for a resume: {e}",
				downloaded_on_disk(&tmp),
				dest.display()
			);
			Err(e)
		}
		Err(StreamFailure::Discard(e)) => {
			drop(file);
			let _ = tokio::fs::remove_file(&tmp).await;
			Err(e)
		}
	}
}

fn downloaded_on_disk(path: &Path) -> u64 {
	std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}
#[cfg(test)]
mod download_tests {
	use super::download_model_file;
	use sha2::{Digest, Sha256};
	use std::io::{Read, Write};
	use std::sync::atomic::AtomicBool;
	use std::sync::Arc;

	/// Loopback HTTP server that hands the request head to a callback so
	/// tests can inspect headers, then serves a canned response.
	fn serve_inspecting(respond: impl FnOnce(&str) -> Vec<u8> + Send + 'static) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				// read just the request head (until \r\n\r\n); never read
				// to EOF - pooled clients keep the connection open
				let mut request = String::new();
				loop {
					let mut byte = [0u8; 1];
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0] as char);
					if request.ends_with("\r\n\r\n") {
						break;
					}
				}
				let response = respond(&request);
				let _ = sock.write_all(&response);
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});
		format!("http://{addr}/model.bin")
	}

	fn sha256_hex(bytes: &[u8]) -> String {
		Sha256::digest(bytes)
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect()
	}

	/// A download destination in its own temp dir (removed on drop).
	fn dest(tag: &str) -> (std::path::PathBuf, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		(dir.path().join(format!("{tag}.bin")), dir)
	}

	#[tokio::test]
	async fn download_sends_bearer_token_when_configured() {
		let body = vec![1u8; 100];
		let digest = sha256_hex(&body);
		let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
		let seen_writer = seen.clone();
		let url = serve_inspecting(move |request| {
			*seen_writer.lock().unwrap() = request.to_string();
			format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
				.into_bytes()
				.into_iter()
				.chain(body.clone())
				.collect()
		});
		let (dest, _dir) = dest("auth");
		let cancel = Arc::new(AtomicBool::new(false));
		download_model_file(
			&url,
			&dest,
			100,
			&digest,
			"hf_token_123",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect("download ok");
		let request = seen.lock().unwrap().clone();
		assert!(
			request
				.to_lowercase()
				.contains("authorization: bearer hf_token_123"),
			"bearer token sent: {request}"
		);
	}

	#[tokio::test]
	async fn download_omits_bearer_header_when_empty() {
		let body = vec![2u8; 50];
		let digest = sha256_hex(&body);
		let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
		let seen_writer = seen.clone();
		let url = serve_inspecting(move |request| {
			*seen_writer.lock().unwrap() = request.to_string();
			format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
				.into_bytes()
				.into_iter()
				.chain(body.clone())
				.collect()
		});
		let (dest, _dir) = dest("anon");
		let cancel = Arc::new(AtomicBool::new(false));
		download_model_file(&url, &dest, 50, &digest, "", &cancel, &mut |_| {})
			.await
			.expect("download ok");
		let request = seen.lock().unwrap().clone();
		assert!(
			!request.to_lowercase().contains("authorization:"),
			"no auth header for anonymous download: {request}"
		);
	}

	#[tokio::test]
	async fn download_reports_indeterminate_progress_without_content_length() {
		// progress is only reported past ~2 MB, so exceed it; the digest
		// is computed before the body moves into the server closure
		let body = vec![3u8; 3_000_000];
		let digest = sha256_hex(&body);
		// chunked transfer, no Content-Length
		let url = serve_inspecting(move |_| {
			let mut out = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
			for chunk in body.chunks(1_000_000) {
				out.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
				out.extend_from_slice(chunk);
				out.extend_from_slice(b"\r\n");
			}
			out.extend_from_slice(b"0\r\n\r\n");
			out
		});
		let (dest, _dir) = dest("indeterminate");
		let cancel = Arc::new(AtomicBool::new(false));
		let mut progress = Vec::new();
		download_model_file(&url, &dest, 3_000_000, &digest, "", &cancel, &mut |p| {
			progress.push(p)
		})
		.await
		.expect("download ok");
		assert!(
			progress.iter().any(|p| *p < 0.0),
			"indeterminate sentinel reported when the total is unknown: {progress:?}"
		);
	}

	#[tokio::test]
	async fn download_fails_fast_when_content_length_differs_from_spec() {
		let body = vec![4u8; 500];
		let url = serve_inspecting(move |_| {
			// server promises 500 bytes but the spec says 1000: the
			// mismatch must be caught from the headers, not after the body
			format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
				.into_bytes()
				.into_iter()
				.chain(body.clone())
				.collect()
		});
		let (dest, _dir) = dest("failfast");
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			1000,
			&sha256_hex(&[4u8; 500]),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("must fail");
		assert!(
			err.contains("server says 500 bytes, expected 1000"),
			"unexpected error: {err}"
		);
		assert!(!dest.exists(), "no file on early rejection");
	}
}
#[cfg(test)]
mod resume_tests {
	use super::{download_model_file, part_path};
	use sha2::{Digest, Sha256};
	use std::io::{Read, Write};
	use std::sync::atomic::AtomicBool;
	use std::sync::Arc;

	fn sha256_hex(bytes: &[u8]) -> String {
		Sha256::digest(bytes)
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect()
	}

	/// Serve the body honoring a Range request (like HuggingFace does).
	fn serve_ranged(
		body: Vec<u8>,
		saw_range: std::sync::Arc<std::sync::Mutex<Option<String>>>,
	) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				let mut request = String::new();
				loop {
					let mut byte = [0u8; 1];
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0] as char);
					if request.ends_with("\r\n\r\n") {
						break;
					}
				}
				let range = request
					.lines()
					.find(|l| l.to_lowercase().starts_with("range:"))
					.map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string());
				*saw_range.lock().unwrap() = range.clone();
				let mut content_range = String::new();
				let (status, slice): (&str, &[u8]) = match range
					.as_deref()
					.and_then(|r| r.strip_prefix("bytes=").and_then(|r| r.split('-').next()))
					.and_then(|start| start.parse::<usize>().ok())
				{
					Some(start) if start < body.len() => {
						// a real 206 advertises the slice it is serving
						content_range = format!(
							"Content-Range: bytes {}-{}/{}\r\n",
							start,
							body.len() - 1,
							body.len()
						);
						("206 Partial Content", &body[start..])
					}
					Some(_) => ("416 Range Not Satisfiable", &[]),
					None => ("200 OK", &body),
				};
				let head = format!(
					"HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n{content_range}\r\n",
					slice.len()
				);
				let _ = sock.write_all(head.as_bytes());
				let _ = sock.write_all(slice);
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});
		format!("http://{addr}/model.bin")
	}

	#[tokio::test]
	async fn resumes_a_partial_file_with_a_range_request() {
		let body = vec![7u8; 3000];
		let digest = sha256_hex(&body);
		let saw_range = std::sync::Arc::new(std::sync::Mutex::new(None));
		let url = serve_ranged(body.clone(), saw_range.clone());
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		let tmp = part_path(&dest);

		// a stalled download left the first 1000 bytes staged
		std::fs::write(&tmp, &body[..1000]).expect("stage prefix");

		let cancel = Arc::new(AtomicBool::new(false));
		download_model_file(
			&url,
			&dest,
			body.len() as u64,
			&digest,
			"",
			&cancel,
			&mut |p| {
				assert!(p >= 0.0, "progress stays valid on resume");
			},
		)
		.await
		.expect("resumed download");

		assert_eq!(
			saw_range.lock().unwrap().as_deref(),
			Some("bytes=1000-"),
			"Range header sent for the staged prefix"
		);
		assert_eq!(
			std::fs::read(&dest).unwrap(),
			body,
			"file assembled correctly"
		);
	}

	/// Promise `body.len()` bytes but hang up after `sent` of them, the way
	/// a dropped connection or a quit mid-transfer looks to the client.
	fn serve_cut_off(body: Vec<u8>, sent: usize) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				// read the whole request head: closing with unread bytes
				// would send a RST, which can discard data in flight
				let mut request = Vec::new();
				let mut byte = [0u8; 1];
				while !request.ends_with(b"\r\n\r\n") {
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0]);
				}
				let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
				let _ = sock.write_all(head.as_bytes());
				let _ = sock.write_all(&body[..sent]);
				let _ = sock.flush();
				// a clean FIN mid-body: the client sees the stream end early
				let _ = sock.shutdown(std::net::Shutdown::Write);
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});
		format!("http://{addr}/model.bin")
	}

	#[tokio::test]
	async fn an_interrupted_download_keeps_its_part_file_and_resumes() {
		let body: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
		let digest = sha256_hex(&body);
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		let tmp = part_path(&dest);
		let cancel = Arc::new(AtomicBool::new(false));

		// the connection drops after 1200 bytes: the attempt fails, but the
		// staged prefix must survive for the next attempt (or launch)
		let url = serve_cut_off(body.clone(), 1200);
		let err = download_model_file(
			&url,
			&dest,
			body.len() as u64,
			&digest,
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("a cut-off transfer must fail");
		assert!(
			err.contains("interrupted") || err.contains("incomplete"),
			"unexpected error: {err}"
		);
		assert!(!dest.exists(), "no final file from a failed transfer");
		assert_eq!(
			std::fs::read(&tmp).expect("the .part file is kept"),
			body[..1200],
			"the kept prefix is exactly what arrived"
		);

		// the retry continues from the kept prefix with a Range request
		let saw_range = std::sync::Arc::new(std::sync::Mutex::new(None));
		let url = serve_ranged(body.clone(), saw_range.clone());
		download_model_file(
			&url,
			&dest,
			body.len() as u64,
			&digest,
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect("resumed download");
		assert_eq!(saw_range.lock().unwrap().as_deref(), Some("bytes=1200-"));
		assert_eq!(std::fs::read(&dest).unwrap(), body);
		assert!(!tmp.exists(), "the staging file is consumed on success");
	}

	#[tokio::test]
	async fn integrity_failures_and_cancellation_discard_the_part_file() {
		let body = vec![5u8; 2000];
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		let cancel = Arc::new(AtomicBool::new(false));
		let saw_range = std::sync::Arc::new(std::sync::Mutex::new(None));
		let url = serve_ranged(body.clone(), saw_range);
		let err = download_model_file(
			&url,
			&dest,
			body.len() as u64,
			"deadbeef",
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("hash mismatch");
		assert!(err.contains("integrity"), "unexpected error: {err}");
		assert!(
			!part_path(&dest).exists(),
			"corrupt bytes are not resumable"
		);

		cancel.store(true, std::sync::atomic::Ordering::Relaxed);
		let saw_range = std::sync::Arc::new(std::sync::Mutex::new(None));
		let url = serve_ranged(body.clone(), saw_range);
		let err = download_model_file(
			&url,
			&dest,
			body.len() as u64,
			&sha256_hex(&body),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("cancelled");
		assert!(err.contains("cancelled"), "unexpected error: {err}");
		assert!(
			!part_path(&dest).exists(),
			"a user cancel frees the disk space"
		);
	}

	#[tokio::test]
	async fn restarts_when_the_partial_file_is_oversized() {
		let body = vec![9u8; 1500];
		let digest = sha256_hex(&body);
		let saw_range = std::sync::Arc::new(std::sync::Mutex::new(None));
		let url = serve_ranged(body.clone(), saw_range.clone());
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");

		// a .part longer than the whole file is junk from another state:
		// no resume is attempted, the download restarts from zero
		std::fs::write(part_path(&dest), vec![0u8; 2000]).expect("stage oversized junk");
		let cancel = Arc::new(AtomicBool::new(false));
		download_model_file(
			&url,
			&dest,
			body.len() as u64,
			&digest,
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect("clean restart");
		assert_eq!(
			saw_range.lock().unwrap().as_deref(),
			None,
			"no Range request for an oversized .part"
		);
		assert_eq!(std::fs::read(&dest).unwrap(), body);
	}

	#[tokio::test]
	async fn restarts_when_the_server_ignores_range() {
		// a server (or proxy) without Range support answers 200 with the
		// full body: the staged prefix must be dropped, not stitched onto it
		let body: Vec<u8> = (0..1500u32).map(|i| (i % 7) as u8).collect();
		let digest = sha256_hex(&body);
		let saw_range = std::sync::Arc::new(std::sync::Mutex::new(None));
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		let served = body.clone();
		let seen = saw_range.clone();
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				let mut request = String::new();
				let mut byte = [0u8; 1];
				while !request.ends_with("\r\n\r\n") {
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0] as char);
				}
				*seen.lock().unwrap() = request
					.lines()
					.find(|l| l.to_lowercase().starts_with("range:"))
					.map(|l| l.to_string());
				let head = format!(
					"HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
					served.len()
				);
				let _ = sock.write_all(head.as_bytes());
				let _ = sock.write_all(&served);
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		std::fs::write(part_path(&dest), b"stale prefix from another transfer")
			.expect("stage prefix");
		let cancel = Arc::new(AtomicBool::new(false));
		download_model_file(
			&format!("http://{addr}/model.bin"),
			&dest,
			body.len() as u64,
			&digest,
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect("restart from the full body");
		assert!(
			saw_range.lock().unwrap().is_some(),
			"a resume was attempted"
		);
		assert_eq!(
			std::fs::read(&dest).unwrap(),
			body,
			"prefix not stitched on"
		);
	}

	/// Serve one canned response verbatim, for hand-built 206 heads
	/// (chunked suffixes, wrong ranges, bogus totals).
	fn serve_raw(response: Vec<u8>) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				let mut request = String::new();
				loop {
					let mut byte = [0u8; 1];
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0] as char);
					if request.ends_with("\r\n\r\n") {
						break;
					}
				}
				let _ = sock.write_all(&response);
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});
		format!("http://{addr}/model.bin")
	}

	/// Encode `body` as an HTTP/1.1 chunked transfer: no length is
	/// derivable from the head, the way streamed 206 suffixes arrive.
	fn chunked(body: &[u8]) -> Vec<u8> {
		let mut out = Vec::new();
		for part in body.chunks(1024) {
			out.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
			out.extend_from_slice(part);
			out.extend_from_slice(b"\r\n");
		}
		out.extend_from_slice(b"0\r\n\r\n");
		out
	}

	#[tokio::test]
	async fn resumes_a_chunked_206_with_a_content_range_total() {
		// The suffix arrives chunked (no Content-Length), so the
		// whole-file size is only declared by Content-Range. A valid
		// resume must not be mistaken for a size mismatch.
		let body = b"abcdef".to_vec();
		let digest = sha256_hex(&body);
		let cancel = Arc::new(AtomicBool::new(false));

		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		std::fs::write(part_path(&dest), &body[..3]).expect("stage prefix");
		let mut response = b"HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nContent-Range: bytes 3-5/6\r\n\r\n".to_vec();
		response.extend_from_slice(&chunked(&body[3..]));
		let url = serve_raw(response);
		download_model_file(&url, &dest, 6, &digest, "", &cancel, &mut |_| {})
			.await
			.expect("chunked 206 resume");
		assert_eq!(std::fs::read(&dest).unwrap(), body);
		assert!(!part_path(&dest).exists(), "staging file consumed");

		// the no-hash mode (empty pinned sha) resumes the same way
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		std::fs::write(part_path(&dest), &body[..3]).expect("stage prefix");
		let mut response = b"HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nContent-Range: bytes 3-5/6\r\n\r\n".to_vec();
		response.extend_from_slice(&chunked(&body[3..]));
		let url = serve_raw(response);
		download_model_file(&url, &dest, 6, "", "", &cancel, &mut |_| {})
			.await
			.expect("chunked 206 resume without a pinned hash");
		assert_eq!(std::fs::read(&dest).unwrap(), body);
	}

	#[tokio::test]
	async fn resumes_a_206_that_carries_a_content_length() {
		// the classic shape: 206 with a Content-Length for the suffix
		// and a Content-Range total for the whole file
		let body = b"0123456789".to_vec();
		let digest = sha256_hex(&body);
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		std::fs::write(part_path(&dest), &body[..4]).expect("stage prefix");
		let mut response =
			b"HTTP/1.1 206 Partial Content\r\nContent-Length: 6\r\nContent-Range: bytes 4-9/10\r\n\r\n".to_vec();
		response.extend_from_slice(&body[4..]);
		let url = serve_raw(response);
		let cancel = Arc::new(AtomicBool::new(false));
		download_model_file(&url, &dest, 10, &digest, "", &cancel, &mut |_| {})
			.await
			.expect("content-length 206 resume");
		assert_eq!(std::fs::read(&dest).unwrap(), body);
	}

	#[tokio::test]
	async fn rejects_a_206_that_resumes_at_the_wrong_offset() {
		// the staged prefix is "abc" but the server restarts at 0:
		// stitching would corrupt the file, so nothing may be appended
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		let tmp = part_path(&dest);
		std::fs::write(&tmp, b"abc").expect("stage prefix");
		let mut response = b"HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nContent-Range: bytes 0-5/6\r\n\r\n".to_vec();
		response.extend_from_slice(&chunked(b"abcdef"));
		let url = serve_raw(response);
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			6,
			&sha256_hex(b"abcdef"),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("a mismatched range must be rejected");
		assert!(
			err.contains("resume position mismatch"),
			"unexpected error: {err}"
		);
		assert!(
			!tmp.exists(),
			"an invalid range is never appended: staged bytes are discarded"
		);
		assert!(!dest.exists());
	}

	#[tokio::test]
	async fn rejects_a_206_whose_declared_total_conflicts_with_the_catalog() {
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		let tmp = part_path(&dest);
		std::fs::write(&tmp, b"abc").expect("stage prefix");
		let mut response = b"HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nContent-Range: bytes 3-5/99\r\n\r\n".to_vec();
		response.extend_from_slice(&chunked(b"def"));
		let url = serve_raw(response);
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			6,
			&sha256_hex(b"abcdef"),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("a contradictory total must be rejected up front");
		assert!(
			err.contains("server says 99 bytes, expected 6"),
			"unexpected error: {err}"
		);
		assert!(!tmp.exists(), "the unusable staged bytes are discarded");
		assert!(!dest.exists());
	}

	#[tokio::test]
	async fn a_truncated_resumed_suffix_keeps_the_part_file() {
		// promised bytes 3-5/6, the stream ends after "d": a short file
		// is still a valid prefix worth resuming
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		let tmp = part_path(&dest);
		std::fs::write(&tmp, b"abc").expect("stage prefix");
		let mut response = b"HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nContent-Range: bytes 3-5/6\r\n\r\n".to_vec();
		response.extend_from_slice(&chunked(b"d"));
		let url = serve_raw(response);
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			6,
			&sha256_hex(b"abcdef"),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("a truncated suffix must fail");
		assert!(err.contains("got 4 of 6 bytes"), "unexpected error: {err}");
		assert_eq!(
			std::fs::read(&tmp).expect("part kept"),
			b"abcd",
			"the arrived prefix is kept for the next resume"
		);
		assert!(!dest.exists());
	}

	#[tokio::test]
	async fn a_resumed_download_with_a_corrupt_suffix_fails_its_integrity_check() {
		// sizes line up (3 + 3 = 6) but the suffix bytes are wrong: the
		// hash over prefix + suffix must catch it and discard the part
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		let tmp = part_path(&dest);
		std::fs::write(&tmp, b"abc").expect("stage prefix");
		let mut response = b"HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nContent-Range: bytes 3-5/6\r\n\r\n".to_vec();
		response.extend_from_slice(&chunked(b"XXX"));
		let url = serve_raw(response);
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			6,
			&sha256_hex(b"abcdef"),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("a corrupt suffix must fail the hash check");
		assert!(err.contains("integrity"), "unexpected error: {err}");
		assert!(!tmp.exists(), "corrupt bytes are not resumable");
		assert!(!dest.exists());
	}

	#[tokio::test]
	async fn rejects_an_unsolicited_206_without_a_staged_prefix() {
		// no Range was sent (nothing to resume), yet the server answers
		// 206: the body is partial and must not be treated as the file
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		let mut response = b"HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 0-2/6\r\n\r\n".to_vec();
		response.extend_from_slice(b"abc");
		let url = serve_raw(response);
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			6,
			&sha256_hex(b"abcdef"),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("an unsolicited 206 must be rejected");
		assert!(
			err.contains("without a range request"),
			"unexpected error: {err}"
		);
		assert!(!part_path(&dest).exists());
		assert!(!dest.exists());
	}

	#[tokio::test]
	async fn rejects_a_206_with_a_malformed_content_range() {
		let dir = tempfile::tempdir().expect("tempdir");
		let dest = dir.path().join("model.bin");
		let tmp = part_path(&dest);
		std::fs::write(&tmp, b"abc").expect("stage prefix");
		let mut response = b"HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nContent-Range: bytes 3-5/not-a-number\r\n\r\n".to_vec();
		response.extend_from_slice(&chunked(b"def"));
		let url = serve_raw(response);
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			6,
			&sha256_hex(b"abcdef"),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("a malformed Content-Range must be rejected");
		assert!(
			err.contains("malformed Content-Range"),
			"unexpected error: {err}"
		);
		assert!(
			!tmp.exists(),
			"nothing is appended when the range cannot be validated"
		);
		assert!(!dest.exists());
	}
}
#[cfg(test)]
mod hf_cache_tests {
	use super::{hf_cache_model_path, LLM_MODELS};

	#[test]
	fn hf_cache_layout_resolves_and_picks_the_newest_snapshot() {
		let dir = tempfile::tempdir().expect("tempdir");
		let spec = &LLM_MODELS[0];
		let repo_dir = dir
			.path()
			.join(format!("models--{}", spec.repo.replace('/', "--")))
			.join("snapshots");

		// two snapshot revisions; only one carries the file
		let old_rev = repo_dir.join("aaaa");
		let new_rev = repo_dir.join("bbbb");
		std::fs::create_dir_all(&old_rev).unwrap();
		std::fs::create_dir_all(&new_rev).unwrap();
		std::fs::write(old_rev.join(spec.filename), b"old").unwrap();
		std::fs::write(new_rev.join(spec.filename), b"new").unwrap();
		// only one revision has the file: deterministic resolution
		std::fs::remove_file(old_rev.join(spec.filename)).unwrap();

		let found = hf_cache_model_path(dir.path(), spec).expect("resolved");
		assert_eq!(found, new_rev.join(spec.filename));

		// (the empty old_rev snapshot exercises the skip path already)
		assert_eq!(
			hf_cache_model_path(dir.path(), spec),
			Some(new_rev.join(spec.filename))
		);
		// no snapshot with the file -> None
		std::fs::remove_file(new_rev.join(spec.filename)).unwrap();
		assert_eq!(hf_cache_model_path(dir.path(), spec), None);
	}

	#[test]
	fn endpoint_resolution_setting_env_default_precedence() {
		use super::super::ai_settings::resolve_hf_endpoint;
		let default = "https://huggingface.co";
		// setting wins over everything, trimmed
		assert_eq!(
			resolve_hf_endpoint(" https://hf-mirror.com/ ", Some("https://other.example")),
			"https://hf-mirror.com"
		);
		// empty setting falls to the env var
		assert_eq!(
			resolve_hf_endpoint("", Some("https://hf-mirror.com/")),
			"https://hf-mirror.com"
		);
		// blank-only setting counts as empty
		assert_eq!(
			resolve_hf_endpoint("   ", Some("https://hf-mirror.com")),
			"https://hf-mirror.com"
		);
		// neither set: the default
		assert_eq!(resolve_hf_endpoint("", None), default);
	}
}
#[cfg(test)]
mod cache_storage_tests {
	use super::{
		hf_blob_path, hf_cache_model_path, materialize_snapshot, migrate_one, part_path,
		remove_cached_model, LLM_MODELS,
	};
	use sha2::{Digest, Sha256};

	fn sha256_hex(bytes: &[u8]) -> String {
		Sha256::digest(bytes)
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect()
	}

	/// A spec-shaped fixture whose pinned sha AND size match `content`,
	/// so migration/removal identify it like a real catalog model.
	fn spec_for(content: &[u8]) -> super::ModelSpec {
		let mut spec = LLM_MODELS[0].clone();
		spec.sha256 = Box::leak(sha256_hex(content).into_boxed_str());
		spec.size_bytes = content.len() as u64;
		spec
	}

	/// [`spec_for`]; kept as a separate name where the pinned size is
	/// the point of the test (content identification).
	fn pinned_spec(content: &[u8]) -> super::ModelSpec {
		spec_for(content)
	}

	#[test]
	fn materialize_publishes_a_blob_and_resolves_through_the_snapshot() {
		let cache = tempfile::tempdir().expect("tempdir");
		let content = b"model bytes";
		let spec = spec_for(content);
		let blob = hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, content).unwrap();

		materialize_snapshot(cache.path(), &spec).expect("materialize");
		// idempotent
		materialize_snapshot(cache.path(), &spec).expect("materialize again");

		let found = hf_cache_model_path(cache.path(), &spec).expect("resolved");
		assert_eq!(std::fs::read(&found).unwrap(), content);
	}

	#[test]
	fn materialize_repairs_a_snapshot_link_that_does_not_resolve() {
		let cache = tempfile::tempdir().expect("tempdir");
		let content = b"model bytes";
		let spec = spec_for(content);
		let blob = hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, content).unwrap();
		let snapshot = blob
			.parent()
			.unwrap()
			.parent()
			.unwrap()
			.join("snapshots")
			.join(spec.sha256);
		std::fs::create_dir_all(&snapshot).unwrap();
		let link = snapshot.join(spec.filename);
		// a dead link: on Windows the `/`-separated target older builds
		// wrote (it never resolves there), elsewhere a missing blob name
		#[cfg(target_os = "windows")]
		if std::os::windows::fs::symlink_file(format!("../../blobs/{}", spec.sha256), &link)
			.is_err()
		{
			return; // no symlink privilege: no dead link can exist
		}
		#[cfg(target_family = "unix")]
		std::os::unix::fs::symlink("../../blobs/missing", &link).unwrap();
		assert!(!link.is_file(), "precondition: the link is dead");

		materialize_snapshot(cache.path(), &spec).expect("materialize");
		let found = hf_cache_model_path(cache.path(), &spec).expect("resolved");
		assert_eq!(std::fs::read(&found).unwrap(), content);
	}

	#[test]
	fn migration_moves_verified_files_and_dedupes_existing_blobs() {
		let dir = tempfile::tempdir().expect("tempdir");
		let models = dir.path().join("models");
		let cache = dir.path().join("hub");
		std::fs::create_dir_all(&models).unwrap();

		let good = b"good model content";
		let good_spec = spec_for(good);
		let good_app = models.join(good_spec.filename);
		std::fs::write(&good_app, good).unwrap();

		// wrong-content file: must stay in the app dir untouched
		let mut bad_spec = spec_for(b"different bytes");
		// pin bad_spec's sha to something the file does NOT have

		bad_spec.sha256 = "deadbeef";
		bad_spec.filename = "stale-file.gguf";
		let bad_app = models.join(bad_spec.filename);
		std::fs::write(&bad_app, b"stale content").unwrap();

		// pre-existing blob: the app copy is redundant and just removed
		let mut dup_spec = spec_for(b"already cached");
		dup_spec.filename = "dup.gguf";
		let dup_blob = hf_blob_path(&cache, &dup_spec);
		std::fs::create_dir_all(dup_blob.parent().unwrap()).unwrap();
		std::fs::write(&dup_blob, b"already cached").unwrap();
		std::fs::write(models.join(dup_spec.filename), b"already cached").unwrap();

		migrate_one(&models, &cache, &good_spec);
		migrate_one(&models, &cache, &bad_spec);
		migrate_one(&models, &cache, &dup_spec);

		// good: moved into the blob, published, app copy gone
		assert!(!good_app.exists(), "app copy removed after migration");
		assert_eq!(
			std::fs::read(hf_blob_path(&cache, &good_spec)).unwrap(),
			good
		);
		assert!(hf_cache_model_path(&cache, &good_spec).is_some());

		// bad: left alone (content does not match the pin)
		assert!(bad_app.exists(), "unverifiable file stays in the app dir");
		assert!(!hf_blob_path(&cache, &bad_spec).exists());

		// dup: blob already present, app copy dropped, snapshot exists
		assert!(!models.join(dup_spec.filename).exists());
		assert!(hf_cache_model_path(&cache, &dup_spec).is_some());
	}

	/// The migration regressions share one shape: an app-dir source,
	/// a cache the test corrupts/blocks, and assertions that a usable
	/// copy survives every injected failure.
	fn migration_fixture(
		_tag: &str,
	) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
		let dir = tempfile::tempdir().expect("tempdir");
		let models = dir.path().join("models");
		let cache = dir.path().join("hub");
		std::fs::create_dir_all(&models).expect("models dir");
		(dir, models, cache)
	}

	#[test]
	fn a_corrupt_cached_blob_never_costs_the_good_app_copy() {
		// A hash-named blob whose bytes do not match the pin must
		// be treated as absent and replaced, never trusted by filename
		// - deleting the good app copy on its word would leave the
		// user with a corrupt, load-failing model
		let (_dir, models, cache) = migration_fixture("corrupt");
		let good = b"good model content";
		let spec = pinned_spec(good);
		let app = models.join(spec.filename);
		std::fs::write(&app, good).unwrap();

		// the corrupt blob is fully published: same length as the pin
		// (only the streamed sha256 can tell it apart) plus a live
		// snapshot link - exactly the state the filename-trusting
		// ordering leaves behind
		let blob = hf_blob_path(&cache, &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, b"badd model content").unwrap();
		materialize_snapshot(&cache, &spec).expect("corrupt snapshot setup");

		migrate_one(&models, &cache, &spec);

		assert_eq!(
			std::fs::read(&blob).unwrap(),
			good.to_vec(),
			"the corrupt blob is replaced by the verified app content"
		);
		let found = hf_cache_model_path(&cache, &spec).expect("the snapshot resolves");
		assert_eq!(
			std::fs::read(&found).unwrap(),
			good.to_vec(),
			"the model ends usable"
		);
		assert!(
			!app.exists(),
			"the app copy goes only once the destination holds verified bytes"
		);
		assert!(!part_path(&blob).exists(), "no staging junk is left");
	}

	#[cfg(target_family = "unix")]
	#[test]
	fn an_unverifiable_blob_never_deletes_the_app_copy() {
		// a blob that cannot be READ is uncertainty, not validity: the
		// app copy is the only provably-good copy and must survive
		use std::os::unix::fs::PermissionsExt;
		let (_dir, models, cache) = migration_fixture("unreadable");
		let good = b"good model content";
		let spec = pinned_spec(good);
		let app = models.join(spec.filename);
		std::fs::write(&app, good).unwrap();

		let blob = hf_blob_path(&cache, &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		// same length as the pin so the size short-circuit cannot answer
		std::fs::write(&blob, b"locked-away-bytes!").unwrap();
		let mut perms = std::fs::metadata(&blob).unwrap().permissions();
		perms.set_mode(0o000);
		std::fs::set_permissions(&blob, perms).unwrap();

		migrate_one(&models, &cache, &spec);

		assert!(
			app.is_file(),
			"the app copy is never dropped on an unverifiable blob"
		);
		assert_eq!(std::fs::read(&app).unwrap(), good.to_vec());
		let mut perms = std::fs::metadata(&blob).unwrap().permissions();
		perms.set_mode(0o644);
		std::fs::set_permissions(&blob, perms).unwrap();
		assert!(
			!part_path(&blob).exists(),
			"no staging junk beside the retained blob"
		);
	}

	#[test]
	fn migration_publishes_the_snapshot_before_dropping_the_app_copy() {
		// A snapshot that cannot be created (here: a regular file
		// squatting on the revision dir) must leave the app copy in
		// place - a bare blob is not discoverable by the resolver
		let (_dir, models, cache) = migration_fixture("blocked");
		let good = b"good model content";
		let spec = pinned_spec(good);
		let app = models.join(spec.filename);
		std::fs::write(&app, good).unwrap();

		let repo = cache.join(format!("models--{}", spec.repo.replace('/', "--")));
		std::fs::create_dir_all(repo.join("snapshots")).unwrap();
		std::fs::write(repo.join("snapshots").join(spec.sha256), b"not a dir").unwrap();

		migrate_one(&models, &cache, &spec);

		// the failure keeps a usable copy: the untouched app file...
		assert_eq!(
			std::fs::read(&app).unwrap(),
			good.to_vec(),
			"the app copy survives a snapshot-publishing failure"
		);
		// ...plus the already-verified blob as retry fodder
		assert_eq!(
			std::fs::read(hf_blob_path(&cache, &spec)).unwrap(),
			good.to_vec()
		);
		assert!(
			hf_cache_model_path(&cache, &spec).is_none(),
			"no snapshot could be published"
		);
		assert!(!part_path(&hf_blob_path(&cache, &spec)).exists());

		// once the blockage is gone, the retry completes cleanly
		std::fs::remove_file(repo.join("snapshots").join(spec.sha256)).unwrap();
		migrate_one(&models, &cache, &spec);
		assert!(!app.exists(), "the retry retires the app copy");
		assert_eq!(
			std::fs::read(hf_cache_model_path(&cache, &spec).expect("resolves")).unwrap(),
			good.to_vec()
		);
	}

	#[test]
	fn migration_heals_a_missing_snapshot_when_the_app_copy_is_already_gone() {
		// the previous ordering renamed the app copy into the blob and
		// only then failed to publish the snapshot, leaving nothing
		// discoverable; the fixed migration repairs that state instead
		// of skipping it (no app file to act on)
		let (_dir, models, cache) = migration_fixture("heal");
		let good = b"good model content";
		let spec = pinned_spec(good);
		let blob = hf_blob_path(&cache, &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, good).unwrap();

		migrate_one(&models, &cache, &spec);

		let found =
			hf_cache_model_path(&cache, &spec).expect("the verified blob becomes discoverable");
		assert_eq!(std::fs::read(&found).unwrap(), good.to_vec());

		// a blob that fails verification gets no snapshot: linking it
		// would advertise corrupt bytes as a usable model
		let mut corrupt = pinned_spec(b"entirely elsewhere");
		corrupt.filename = "corrupt.gguf";
		let corrupt_blob = hf_blob_path(&cache, &corrupt);
		std::fs::create_dir_all(corrupt_blob.parent().unwrap()).unwrap();
		std::fs::write(&corrupt_blob, b"junk bytes").unwrap();
		migrate_one(&models, &cache, &corrupt);
		assert!(
			hf_cache_model_path(&cache, &corrupt).is_none(),
			"corrupt bytes are never published as a model"
		);
	}

	#[test]
	fn an_already_resolving_cache_skips_blob_verification_at_startup() {
		// no app file and a snapshot that already resolves: the cache is
		// in its final state, so startup migration does NOTHING - in
		// particular it must not re-hash the multi-GB blobs of every
		// downloaded model on every launch, which kept the engines from
		// loading for most of a minute. Even a blob whose bytes went bad
		// under the pinned name is left untouched here: with no
		// migration decision to make, verification has nothing to say,
		// and a genuinely bad model is caught at engine-load time.
		let (_dir, models, cache) = migration_fixture("fast-path");
		let spec = pinned_spec(b"original content");
		let blob = hf_blob_path(&cache, &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, b"original content").unwrap();
		materialize_snapshot(&cache, &spec).expect("snapshot");
		// the blob's bytes silently go bad under the pinned name after
		// the snapshot already resolves
		std::fs::write(&blob, b"rotten bytes!!").unwrap();

		migrate_one(&models, &cache, &spec);

		// nothing was rewritten or "repaired", and the model still
		// resolves through the snapshot
		assert_eq!(
			std::fs::read(&blob).unwrap(),
			b"rotten bytes!!".to_vec(),
			"startup must not rewrite cache content it has no migration decision about"
		);
		assert!(hf_cache_model_path(&cache, &spec).is_some());
	}

	#[test]
	fn same_volume_migration_is_complete_and_repeatable() {
		let (_dir, models, cache) = migration_fixture("same");
		let good = b"good model content";
		let spec = pinned_spec(good);
		let app = models.join(spec.filename);
		std::fs::write(&app, good).unwrap();

		migrate_one(&models, &cache, &spec);
		assert!(!app.exists(), "the app copy is retired");
		let blob = hf_blob_path(&cache, &spec);
		assert_eq!(std::fs::read(&blob).unwrap(), good.to_vec());
		let found = hf_cache_model_path(&cache, &spec).expect("the snapshot resolves");
		assert_eq!(std::fs::read(&found).unwrap(), good.to_vec());
		assert!(!part_path(&blob).exists());

		// repeatable: later runs change nothing and fail nothing
		migrate_one(&models, &cache, &spec);
		migrate_one(&models, &cache, &spec);
		assert!(!app.exists());
		assert_eq!(std::fs::read(&blob).unwrap(), good.to_vec());
		assert!(hf_cache_model_path(&cache, &spec).is_some());
	}

	#[test]
	fn cross_volume_migration_copies_verifies_and_publishes() {
		// separate roots stand in for separate volumes; same-volume and
		// cross-volume moves share the temp-copy/verify/publish
		// ordering, so this drives exactly the cross-volume path
		let models_root = tempfile::tempdir().expect("tempdir");
		let cache_root = tempfile::tempdir().expect("tempdir");
		let models = models_root.path().to_path_buf();
		let cache = cache_root.path().join("hub");
		let good = b"good model content";
		let spec = pinned_spec(good);
		let app = models.join(spec.filename);
		std::fs::write(&app, good).unwrap();

		migrate_one(&models, &cache, &spec);
		assert!(!app.exists(), "the app copy is retired");
		assert_eq!(
			std::fs::read(hf_blob_path(&cache, &spec)).unwrap(),
			good.to_vec()
		);
		assert_eq!(
			std::fs::read(hf_cache_model_path(&cache, &spec).expect("resolves")).unwrap(),
			good.to_vec()
		);
	}

	#[test]
	fn an_interrupted_migration_retries_cleanly() {
		// interrupted mid-copy: a partial staging file plus the intact
		// app copy; the retry must converge and leave no junk behind
		let (_dir, models, cache) = migration_fixture("retry-copy");
		let good = b"good model content";
		let spec = pinned_spec(good);
		let app = models.join(spec.filename);
		std::fs::write(&app, good).unwrap();
		let blob = hf_blob_path(&cache, &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(part_path(&blob), &good[..5]).unwrap(); // interrupted staging

		migrate_one(&models, &cache, &spec);
		assert!(!app.exists(), "the retry completes the migration");
		assert_eq!(std::fs::read(&blob).unwrap(), good.to_vec());
		assert!(hf_cache_model_path(&cache, &spec).is_some());
		assert!(
			!part_path(&blob).exists(),
			"no staging junk survives the retry"
		);

		// interrupted between publish and cleanup: the verified blob
		// and the app copy coexist; the retry dedupes them
		let (_dir, models, cache) = migration_fixture("retry-dedup");
		let mut spec2 = pinned_spec(good);
		spec2.filename = "second.gguf";
		let app2 = models.join(spec2.filename);
		std::fs::write(&app2, good).unwrap();
		let blob2 = hf_blob_path(&cache, &spec2);
		std::fs::create_dir_all(blob2.parent().unwrap()).unwrap();
		std::fs::write(&blob2, good).unwrap(); // published, cleanup never ran

		migrate_one(&models, &cache, &spec2);
		assert!(!app2.exists(), "the redundant copy goes on the retry");
		assert_eq!(std::fs::read(&blob2).unwrap(), good.to_vec());
		assert!(hf_cache_model_path(&cache, &spec2).is_some());
	}

	#[cfg(target_family = "unix")]
	#[test]
	fn migration_survives_a_readonly_blobs_dir() {
		// a copy that cannot even start leaves the original untouched
		use std::os::unix::fs::PermissionsExt;
		let (_dir, models, cache) = migration_fixture("ro");
		let good = b"good model content";
		let spec = pinned_spec(good);
		let app = models.join(spec.filename);
		std::fs::write(&app, good).unwrap();
		let blob = hf_blob_path(&cache, &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		let mut perms = std::fs::metadata(blob.parent().unwrap())
			.unwrap()
			.permissions();
		perms.set_mode(0o555);
		std::fs::set_permissions(blob.parent().unwrap(), perms).unwrap();

		migrate_one(&models, &cache, &spec);

		assert_eq!(
			std::fs::read(&app).unwrap(),
			good.to_vec(),
			"the app copy survives the failed copy"
		);
		assert!(!blob.exists());
		assert!(!part_path(&blob).exists(), "no staging junk on failure");
		let mut perms = std::fs::metadata(blob.parent().unwrap())
			.unwrap()
			.permissions();
		perms.set_mode(0o755);
		std::fs::set_permissions(blob.parent().unwrap(), perms).unwrap();
	}

	#[test]
	fn migration_survives_a_directory_squatting_on_the_blob_name() {
		// publishing over a directory fails: the app copy must survive
		// and the squatting entry must not be damaged
		let (_dir, models, cache) = migration_fixture("dir");
		let good = b"good model content";
		let spec = pinned_spec(good);
		let app = models.join(spec.filename);
		std::fs::write(&app, good).unwrap();
		let blob = hf_blob_path(&cache, &spec);
		std::fs::create_dir_all(&blob).unwrap(); // a directory, not a file
		std::fs::write(blob.join("inner.txt"), b"foreign").unwrap();

		migrate_one(&models, &cache, &spec);

		assert_eq!(
			std::fs::read(&app).unwrap(),
			good.to_vec(),
			"the app copy survives the failed publish"
		);
		assert!(
			blob.join("inner.txt").is_file(),
			"the foreign entry is untouched"
		);
		assert!(!part_path(&blob).exists(), "no staging junk");
	}

	#[test]
	fn a_corrupt_source_is_preserved_untouched_and_reported() {
		// the source must BE the pinned content before anything is
		// published under the pinned name; a file that is not (here:
		// same length, one byte off - only the sha256 catches it)
		// stays exactly as it was (the mismatch is reported through
		// the warn! log, the pre-existing reporting path)
		let (_dir, models, cache) = migration_fixture("source");
		let good = b"good model content";
		let spec = pinned_spec(good);
		let app = models.join(spec.filename);
		let corrupt = b"goad model content";
		std::fs::write(&app, corrupt).unwrap();

		migrate_one(&models, &cache, &spec);

		assert_eq!(
			std::fs::read(&app).unwrap(),
			corrupt.to_vec(),
			"the unverifiable source is untouched"
		);
		assert!(
			!hf_blob_path(&cache, &spec).exists(),
			"nothing is published under the pinned name"
		);
		assert!(hf_cache_model_path(&cache, &spec).is_none());
	}

	#[cfg(target_family = "unix")]
	#[test]
	fn remove_prunes_snapshots_and_only_unreferenced_blobs() {
		let cache = tempfile::tempdir().expect("tempdir");
		let content = b"shared model bytes";
		let spec = spec_for(content);
		let blob = hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, content).unwrap();
		materialize_snapshot(cache.path(), &spec).expect("materialize");

		// a second snapshot revision sharing the blob (as another tool
		// would have created it)
		let repo_snapshots = blob.parent().unwrap().parent().unwrap().join("snapshots");
		let other = repo_snapshots.join("realcommit");
		std::fs::create_dir_all(&other).unwrap();
		std::os::unix::fs::symlink(
			std::path::Path::new("../../blobs").join(spec.sha256),
			other.join("different-name.gguf"),
		)
		.unwrap();

		// delete: both snapshot links go, blob kept while referenced
		assert!(remove_cached_model(cache.path(), &spec).expect("remove"));
		assert!(!hf_cache_model_path(cache.path(), &spec).is_some());
		assert!(
			blob.is_file(),
			"blob survives while another snapshot references it"
		);

		// the other tool's link is the only thing holding the blob now
		assert!(!remove_cached_model(cache.path(), &spec).expect("no-op remove"));
		assert!(blob.is_file(), "still referenced: kept");
		// once that link is gone too, the next remove prunes the blob
		std::fs::remove_file(other.join("different-name.gguf")).unwrap();
		assert!(remove_cached_model(cache.path(), &spec).expect("prune remove"));
		assert!(!blob.is_file(), "unreferenced blob is pruned");
	}

	#[cfg(target_family = "unix")]
	#[test]
	fn remove_spares_a_different_revision_with_the_same_filename() {
		// Two revisions carry the same filename; only the pinned
		// content is ours, so only the pinned revision's entry may go
		let cache = tempfile::tempdir().expect("tempdir");
		let content = b"pinned model bytes";
		let spec = pinned_spec(content);
		let blob = hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, content).unwrap();
		materialize_snapshot(cache.path(), &spec).expect("materialize");

		let snapshots = blob.parent().unwrap().parent().unwrap().join("snapshots");
		// a foreign revision with a same-named REGULAR file (different
		// content, different size)
		let regular_rev = snapshots.join("regular-rev");
		std::fs::create_dir_all(&regular_rev).unwrap();
		std::fs::write(
			regular_rev.join(spec.filename),
			b"a foreign revision of the model",
		)
		.unwrap();
		// a foreign revision with a same-named LINK to a different blob
		let link_rev = snapshots.join("link-rev");
		std::fs::create_dir_all(&link_rev).unwrap();
		std::os::unix::fs::symlink(
			std::path::Path::new("../../blobs").join("deadbeef"),
			link_rev.join(spec.filename),
		)
		.unwrap();
		// an unrelated file in the pinned revision: never a candidate
		std::fs::write(snapshots.join(spec.sha256).join("unrelated.txt"), b"notes").unwrap();

		assert!(remove_cached_model(cache.path(), &spec).expect("remove"));
		// the pinned revision's link is gone...
		assert!(!snapshots.join(spec.sha256).join(spec.filename).exists());
		// ...and everything that is not the pinned content survives
		assert_eq!(
			std::fs::read(regular_rev.join(spec.filename)).unwrap(),
			b"a foreign revision of the model".to_vec(),
			"a different revision's same-named file must not be removed by filename alone"
		);
		assert!(
			link_rev.join(spec.filename).symlink_metadata().is_ok(),
			"a different revision's same-named link must survive"
		);
		assert!(
			snapshots.join(spec.sha256).join("unrelated.txt").is_file(),
			"unrelated filenames are untouched"
		);
	}

	#[test]
	fn remove_verifies_regular_file_snapshots_by_content() {
		// the hardlink/copy fallback layout (e.g. Windows): revisions
		// hold same-named REGULAR files - only the pinned content goes
		let cache = tempfile::tempdir().expect("tempdir");
		let content = b"pinned model bytes";
		let spec = pinned_spec(content);
		let blob = hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, content).unwrap();

		let snapshots = blob.parent().unwrap().parent().unwrap().join("snapshots");
		let pinned_rev = snapshots.join("pinned-rev");
		let forged_rev = snapshots.join("forged-rev");
		let shorter_rev = snapshots.join("shorter-rev");
		for rev in [&pinned_rev, &forged_rev, &shorter_rev] {
			std::fs::create_dir_all(rev).unwrap();
		}
		std::fs::write(pinned_rev.join(spec.filename), content).unwrap();
		// same length as the pin, one byte different: only the
		// streamed sha256 can tell them apart
		std::fs::write(forged_rev.join(spec.filename), b"pinned model bytez").unwrap();
		std::fs::write(shorter_rev.join(spec.filename), b"tiny").unwrap();

		assert!(remove_cached_model(cache.path(), &spec).expect("remove"));
		assert!(
			!pinned_rev.join(spec.filename).exists(),
			"a regular file whose streamed hash matches the pin is removed"
		);
		assert!(
			forged_rev.join(spec.filename).is_file(),
			"same size but different content: retained"
		);
		assert!(
			shorter_rev.join(spec.filename).is_file(),
			"a size that contradicts the catalog: retained without hashing"
		);

		// repeat deletion: nothing left that identifies as pinned
		assert!(!remove_cached_model(cache.path(), &spec).expect("no-op remove"));
	}

	#[cfg(target_family = "unix")]
	#[test]
	fn remove_retains_broken_links_that_do_not_target_the_pin() {
		let cache = tempfile::tempdir().expect("tempdir");
		let content = b"pinned model bytes";
		let spec = pinned_spec(content);
		let blob = hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, content).unwrap();
		materialize_snapshot(cache.path(), &spec).expect("materialize");

		let snapshots = blob.parent().unwrap().parent().unwrap().join("snapshots");
		// a revision whose same-named entry is a dead link to a
		// different blob: not the pinned model, so not ours to remove
		let dead = snapshots.join("dead-rev");
		std::fs::create_dir_all(&dead).unwrap();
		std::os::unix::fs::symlink(
			std::path::Path::new("../../blobs").join("0123dead"),
			dead.join(spec.filename),
		)
		.unwrap();
		assert!(
			!dead.join(spec.filename).is_file(),
			"precondition: the link is broken"
		);

		remove_cached_model(cache.path(), &spec).expect("remove");
		assert!(
			dead.join(spec.filename).symlink_metadata().is_ok(),
			"a broken link to a different blob is not the pinned model"
		);
	}

	#[cfg(target_family = "unix")]
	#[test]
	fn an_unreadable_snapshots_dir_skips_removal_and_retains_the_blob() {
		use std::os::unix::fs::PermissionsExt;
		let cache = tempfile::tempdir().expect("tempdir");
		let content = b"pinned model bytes";
		let spec = pinned_spec(content);
		let blob = hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, content).unwrap();
		materialize_snapshot(cache.path(), &spec).expect("materialize");
		let snapshots = blob.parent().unwrap().parent().unwrap().join("snapshots");

		let mut perms = std::fs::metadata(&snapshots).unwrap().permissions();
		perms.set_mode(0o000);
		std::fs::set_permissions(&snapshots, perms).unwrap();
		let removed = remove_cached_model(cache.path(), &spec);
		let mut perms = std::fs::metadata(&snapshots).unwrap().permissions();
		perms.set_mode(0o755);
		std::fs::set_permissions(&snapshots, perms).unwrap();

		// a scan that cannot run is not "no cache entry": uncertainty
		// skips removal (with a warning) instead of failing or pruning
		assert!(
			!removed.expect("uncertainty is not a hard failure"),
			"nothing is removed when the snapshots dir cannot be listed"
		);
		assert!(
			snapshots
				.join(spec.sha256)
				.join(spec.filename)
				.symlink_metadata()
				.is_ok(),
			"the pinned snapshot entry is retained"
		);
		assert!(blob.is_file(), "the blob is never pruned on a failed scan");
	}

	#[cfg(target_family = "unix")]
	#[test]
	fn an_unreadable_revision_dir_retains_the_blob() {
		use std::os::unix::fs::PermissionsExt;
		let cache = tempfile::tempdir().expect("tempdir");
		let content = b"pinned model bytes";
		let spec = pinned_spec(content);
		let blob = hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, content).unwrap();
		materialize_snapshot(cache.path(), &spec).expect("materialize");
		let snapshots = blob.parent().unwrap().parent().unwrap().join("snapshots");

		// a revision another process made unreadable: whether it
		// references the blob is unknowable
		let locked = snapshots.join("locked-rev");
		std::fs::create_dir_all(&locked).unwrap();
		let mut perms = std::fs::metadata(&locked).unwrap().permissions();
		perms.set_mode(0o000);
		std::fs::set_permissions(&locked, perms).unwrap();

		// our own pinned link is still identified and removed
		assert!(remove_cached_model(cache.path(), &spec).expect("remove"));

		let mut perms = std::fs::metadata(&locked).unwrap().permissions();
		perms.set_mode(0o755);
		std::fs::set_permissions(&locked, perms).unwrap();

		assert!(
			blob.is_file(),
			"an unreadable revision is uncertainty, not absence: the blob must not be pruned"
		);
	}
}

#[cfg(test)]
mod dl_flow_tests {
	use super::*;
	use sha2::{Digest, Sha256};
	use std::sync::Arc;

	fn serve(response: Vec<u8>) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut buf = [0u8; 4096];
				let _ = sock.read(&mut buf); // drain the request head
				let _ = sock.write_all(&response);
				let _ = sock.flush();
				// keep the socket open briefly so the client can read it all
				std::thread::sleep(std::time::Duration::from_millis(500));
			}
		});
		format!("http://{addr}/model.bin")
	}

	fn http(body: &[u8], extra_headers: &str) -> Vec<u8> {
		let mut response = format!(
			"HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{}\r\n",
			body.len(),
			extra_headers
		)
		.into_bytes();
		response.extend_from_slice(body);
		response
	}

	fn sha256_hex(bytes: &[u8]) -> String {
		Sha256::digest(bytes)
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect::<String>()
	}

	fn temp_dest(name: &str) -> (std::path::PathBuf, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		(dir.path().join(name), dir)
	}

	#[tokio::test]
	async fn downloads_and_verifies_a_clean_file() {
		let body = vec![7u8; 100_000];
		let url = serve(http(&body, ""));
		let (dest, _dir) = temp_dest("ok");
		let cancel = Arc::new(AtomicBool::new(false));
		let mut progress = Vec::new();
		download_model_file(
			&url,
			&dest,
			body.len() as u64,
			&sha256_hex(&body),
			"",
			&cancel,
			&mut |p| progress.push(p),
		)
		.await
		.expect("clean download");
		assert!(dest.is_file());
		assert_eq!(std::fs::read(&dest).unwrap(), body);
		assert!(!progress.is_empty(), "progress was reported");
		assert_eq!(*progress.last().unwrap(), 100.0);
	}
	#[tokio::test]
	async fn rejects_a_hash_mismatch() {
		let body = vec![7u8; 10_000];
		let url = serve(http(&body, ""));
		let (dest, _dir) = temp_dest("hash");
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			body.len() as u64,
			"deadbeef",
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("hash mismatch must fail");
		assert!(err.contains("integrity"), "unexpected error: {err}");
		assert!(!dest.exists(), "no file left behind on failure");
	}
	#[tokio::test]
	async fn rejects_a_truncated_transfer() {
		// Content-Length promises more than the body delivers
		let body = vec![1u8; 500];
		let url = serve(http(&body, ""));
		let (dest, _dir) = temp_dest("trunc");
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			100_000,
			&sha256_hex(&body),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("truncated transfer must fail");
		assert!(
			err.contains("incomplete") || err.contains("mismatch"),
			"unexpected error: {err}"
		);
		assert!(!dest.exists());
	}
	#[tokio::test]
	async fn honors_cancellation() {
		let body = vec![3u8; 10_000];
		let url = serve(http(&body, ""));
		let (dest, _dir) = temp_dest("cancel");
		let cancel = Arc::new(AtomicBool::new(false));
		cancel.store(true, std::sync::atomic::Ordering::Relaxed);
		let err = download_model_file(
			&url,
			&dest,
			body.len() as u64,
			&sha256_hex(&body),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("cancelled download must fail");
		assert!(err.contains("cancelled"), "unexpected error: {err}");
		assert!(!dest.exists());
	}
	#[tokio::test]
	async fn rejects_a_size_mismatch() {
		let body = vec![5u8; 1_000];
		let url = serve(http(&body, ""));
		let (dest, _dir) = temp_dest("size");
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			999_999,
			&sha256_hex(&body),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("size mismatch must fail");
		assert!(err.contains("size mismatch"), "unexpected error: {err}");
		assert!(!dest.exists());
	}
}
#[cfg(test)]
mod transport_timeout_tests {
	use super::{checked_next_total, download_with_timeouts, part_path, Timeouts};
	use sha2::{Digest, Sha256};
	use std::io::{Read, Write};
	use std::sync::atomic::AtomicBool;
	use std::sync::Arc;
	use std::time::{Duration, Instant};

	/// Tiny deadlines so deadline paths run in milliseconds: a generous
	/// header/stall budget when the test wants the cancel path, a
	/// 300ms header when it wants the deadline itself, and a fast
	/// cancel poll everywhere.
	fn test_timeouts(header_ms: u64, stall: Duration) -> Timeouts {
		Timeouts {
			header: Duration::from_millis(header_ms),
			stall,
			cancel_poll: Duration::from_millis(20),
		}
	}

	#[test]
	fn huge_chunk_sizes_cannot_overflow_the_byte_counter() {
		assert_eq!(checked_next_total(5, 5), Some(10));
		assert_eq!(checked_next_total(u64::MAX - 4, 4), Some(u64::MAX));
		assert_eq!(
			checked_next_total(u64::MAX - 4, 5),
			None,
			"a lying server cannot wrap the running total"
		);
	}

	fn sha256_hex(bytes: &[u8]) -> String {
		Sha256::digest(bytes)
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect()
	}

	/// Accept the connection, optionally write a canned response, then
	/// hold the socket open in silence: no FIN, so the client sees a
	/// connected server that never starts (or stops) talking.
	fn serve_then_hold(response: Vec<u8>) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				let mut request = Vec::new();
				let mut byte = [0u8; 1];
				while !request.ends_with(b"\r\n\r\n") {
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0]);
				}
				if !response.is_empty() {
					let _ = sock.write_all(&response);
					let _ = sock.flush();
				}
				std::thread::sleep(Duration::from_secs(10));
			}
		});
		format!("http://{addr}/model.bin")
	}

	/// Encode `body` as HTTP/1.1 chunked transfer (no derivable length).
	fn chunked(body: &[u8]) -> Vec<u8> {
		let mut out = Vec::new();
		for part in body.chunks(1024) {
			out.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
			out.extend_from_slice(part);
			out.extend_from_slice(b"\r\n");
		}
		out.extend_from_slice(b"0\r\n\r\n");
		out
	}

	fn dest(tag: &str) -> (std::path::PathBuf, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		(dir.path().join(format!("{tag}.bin")), dir)
	}

	/// Flip the cancel flag from another thread after `delay`, the way
	/// the UI's cancel button lands mid-download.
	fn canceller_after(delay: Duration, cancel: &Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
		let cancel = cancel.clone();
		std::thread::spawn(move || {
			std::thread::sleep(delay);
			cancel.store(true, std::sync::atomic::Ordering::Relaxed);
		})
	}

	#[tokio::test]
	async fn a_silent_server_times_out_waiting_for_headers() {
		// the staged prefix also proves the cleanup policy: a header
		// deadline is transient, so the prefix must survive it
		let url = serve_then_hold(Vec::new());
		let (dest, _dir) = dest("nohdr");
		let tmp = part_path(&dest);
		std::fs::write(&tmp, b"abc").expect("stage prefix");
		let cancel = Arc::new(AtomicBool::new(false));
		let attempt = tokio::time::timeout(
			Duration::from_secs(5),
			download_with_timeouts(
				&url,
				&dest,
				6,
				&sha256_hex(b"abcdef"),
				"",
				&cancel,
				&mut |_| {},
				test_timeouts(300, Duration::from_secs(5)),
			),
		)
		.await
		.expect("the header wait must be bounded");
		let err = attempt.expect_err("a silent server must fail the download");
		assert!(
			err.contains("timed out waiting"),
			"expected a header-deadline error, got: {err}"
		);
		assert_eq!(
			std::fs::read(&tmp).expect("part kept"),
			b"abc".to_vec(),
			"a header deadline is transient: the prefix stays resumable"
		);
		assert!(!dest.exists(), "dest is never touched on error");
	}

	#[tokio::test]
	async fn cancelling_while_headers_are_pending_returns_promptly() {
		let url = serve_then_hold(Vec::new());
		let (dest, _dir) = dest("cancelhdr");
		let tmp = part_path(&dest);
		std::fs::write(&tmp, b"abc").expect("stage prefix");
		let cancel = Arc::new(AtomicBool::new(false));
		let canceller = canceller_after(Duration::from_millis(100), &cancel);
		let start = Instant::now();
		let err = tokio::time::timeout(
			Duration::from_secs(5),
			download_with_timeouts(
				&url,
				&dest,
				6,
				&sha256_hex(b"abcdef"),
				"",
				&cancel,
				&mut |_| {},
				test_timeouts(10_000, Duration::from_secs(5)),
			),
		)
		.await
		.expect("cancel must not wait out the header wait")
		.expect_err("cancelled download must fail");
		assert!(err.contains("cancelled"), "unexpected error: {err}");
		assert!(
			start.elapsed() < Duration::from_secs(1),
			"cancel answered in {:?}, not after the header deadline",
			start.elapsed()
		);
		assert!(!tmp.exists(), "a user cancel discards the staged prefix");
		assert!(!dest.exists());
		canceller.join().unwrap();
	}

	#[tokio::test]
	async fn cancelling_during_a_stalled_body_chunk_returns_promptly() {
		// headers arrive, then the server goes silent mid-body: the
		// cancel must land within the short poll bound, not the stall
		let url = serve_then_hold(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n".to_vec());
		let (dest, _dir) = dest("cancelbody");
		let cancel = Arc::new(AtomicBool::new(false));
		let canceller = canceller_after(Duration::from_millis(150), &cancel);
		let start = Instant::now();
		let err = tokio::time::timeout(
			Duration::from_secs(5),
			download_with_timeouts(
				&url,
				&dest,
				100,
				&sha256_hex(&[0u8; 100]),
				"",
				&cancel,
				&mut |_| {},
				test_timeouts(5_000, Duration::from_secs(10)),
			),
		)
		.await
		.expect("cancel must not wait out the stall timeout")
		.expect_err("cancelled download must fail");
		assert!(err.contains("cancelled"), "unexpected error: {err}");
		assert!(
			start.elapsed() < Duration::from_secs(1),
			"cancel answered in {:?}, not after the stall timeout",
			start.elapsed()
		);
		assert!(
			!part_path(&dest).exists(),
			"a cancel discards the partial body"
		);
		assert!(!dest.exists());
		canceller.join().unwrap();
	}

	#[tokio::test]
	async fn an_oversized_chunked_stream_is_rejected_before_persistence() {
		// chunked with no EOF: the stream must be cut the moment it
		// would outgrow the catalog size, never written past it
		let expected = 2048usize;
		let served = vec![9u8; 3072];
		let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
		// three chunk frames, then the server holds the connection
		for part in served.chunks(1024) {
			response.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
			response.extend_from_slice(part);
			response.extend_from_slice(b"\r\n");
		}
		let url = serve_then_hold(response);
		let (dest, _dir) = dest("oversize");
		let cancel = Arc::new(AtomicBool::new(false));
		let err = tokio::time::timeout(
			Duration::from_secs(5),
			download_with_timeouts(
				&url,
				&dest,
				expected as u64,
				&sha256_hex(&served[..expected]),
				"",
				&cancel,
				&mut |_| {},
				test_timeouts(5_000, Duration::from_secs(10)),
			),
		)
		.await
		.expect("oversize must be rejected without waiting for EOF")
		.expect_err("an oversized stream must fail");
		assert!(
			err.contains("exceeded the expected size"),
			"unexpected error: {err}"
		);
		assert!(
			!part_path(&dest).exists(),
			"oversize bytes are discarded like an integrity failure"
		);
		assert!(!dest.exists());
	}

	#[tokio::test]
	async fn cancelling_during_prefix_hashing_returns_promptly() {
		// a 3 GiB sparse staged prefix takes seconds to hash: a cancel
		// landing mid-hash must abort within the short bound instead
		// of hashing the whole prefix first (sparse: no disk usage)
		let prefix_len: u64 = 3 << 30;
		let suffix = b"tail".to_vec();
		let total = prefix_len + suffix.len() as u64;
		let mut response = format!(
			"HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nContent-Range: bytes {prefix_len}-{}/{}\r\n\r\n",
			total - 1,
			total
		)
		.into_bytes();
		response.extend_from_slice(&chunked(&suffix));
		let url = serve_then_hold(response);
		let (dest, _dir) = dest("cancelhash");
		let tmp = part_path(&dest);
		std::fs::File::create(&tmp)
			.expect("create prefix")
			.set_len(prefix_len)
			.expect("sparse prefix");
		let cancel = Arc::new(AtomicBool::new(false));
		let canceller = canceller_after(Duration::from_millis(100), &cancel);
		let start = Instant::now();
		let err = tokio::time::timeout(
			Duration::from_secs(30),
			download_with_timeouts(
				&url,
				&dest,
				total,
				&sha256_hex(&suffix),
				"",
				&cancel,
				&mut |_| {},
				test_timeouts(5_000, Duration::from_secs(5)),
			),
		)
		.await
		.expect("prefix hashing is bounded by cancellation")
		.expect_err("cancelled download must fail");
		assert!(err.contains("cancelled"), "unexpected error: {err}");
		assert!(
			start.elapsed() < Duration::from_secs(1),
			"cancel answered in {:?}, not after hashing 3 GiB",
			start.elapsed()
		);
		assert!(!tmp.exists(), "a user cancel discards the staged prefix");
		assert!(!dest.exists());
		canceller.join().unwrap();
	}
}
#[test]
fn part_paths_do_not_collide_across_extensions() {
	use std::path::Path;
	let bin = part_path(Path::new("/m/x.bin"));
	let gguf = part_path(Path::new("/m/x.gguf"));
	assert_eq!(bin, Path::new("/m/x.bin.part"));
	assert_eq!(gguf, Path::new("/m/x.gguf.part"));
	assert_ne!(bin, gguf, "staging names must be distinct");
}

#[cfg(test)]
mod sweep_tests {
	use super::{sweep_stale_part_files, LLM_MODELS, PART_MAX_AGE};

	fn touch(path: &std::path::Path, bytes: usize) {
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, vec![0u8; bytes]).unwrap();
	}

	#[test]
	fn startup_sweep_keeps_resumable_parts_and_drops_the_rest() {
		let dir = tempfile::tempdir().expect("tempdir");
		let models = dir.path().join("models");
		let cache = dir.path().join("hub");
		let spec = &LLM_MODELS[0];
		let blobs = cache
			.join(format!("models--{}", spec.repo.replace('/', "--")))
			.join("blobs");

		// a quit mid-download: this is what the next launch resumes
		let resumable = blobs.join(format!("{}.part", spec.sha256));
		touch(&resumable, 1000);
		// a .part under a hash no catalog entry pins (an old pin)
		let stale_pin = blobs.join("deadbeef.part");
		touch(&stale_pin, 10);
		// legacy app-dir staging file (interrupted migration copy)
		let legacy = models.join("whatever.gguf.part");
		touch(&legacy, 10);
		// another tool's repo: not ours to clean
		let foreign = cache
			.join("models--someone--else")
			.join("blobs")
			.join("x.part");
		touch(&foreign, 10);
		// finished blobs are never touched
		let blob = blobs.join(spec.sha256);
		touch(&blob, 10);

		sweep_stale_part_files(&models, &cache);

		assert!(
			resumable.exists(),
			"a fresh catalog .part is kept for resume"
		);
		assert!(!stale_pin.exists(), "a .part nothing can resume is removed");
		assert!(!legacy.exists(), "legacy app-dir staging files are removed");
		assert!(
			foreign.exists(),
			"other tools' cache entries are left alone"
		);
		assert!(blob.exists());

		// once it has sat untouched for too long it is reclaimed
		let old = std::time::SystemTime::now() - PART_MAX_AGE - std::time::Duration::from_secs(60);
		std::fs::File::options()
			.write(true)
			.open(&resumable)
			.unwrap()
			.set_modified(old)
			.unwrap();
		sweep_stale_part_files(&models, &cache);
		assert!(!resumable.exists(), "an abandoned .part is reclaimed");
	}
}
