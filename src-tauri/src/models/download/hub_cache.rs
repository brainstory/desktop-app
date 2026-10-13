//! The HuggingFace hub cache: repo/blob/snapshot paths, legacy migration,
//! and verified cache removal.

use std::path::{Path, PathBuf};

use super::super::catalog::{ModelSpec, LLM_MODELS, STT_MODELS};
use super::stream::part_path;

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

/// Snapshot regular files of `spec` verified against the pin BEFORE the
/// engine-slot claim, keyed by path: `Some(true)` = pinned (removable),
/// `Some(false)` = same name, foreign content, `None` = could not verify
/// (must be retained). Symlinks are not hashed and not listed - the
/// delete-time check for them is a cheap readlink.
pub type VerifiedSnapshotFiles = std::collections::HashMap<std::path::PathBuf, Option<bool>>;

/// Pre-verify (hash) the snapshot files a later [`remove_cached_model`]
/// would need to check, so multi-GB hashing never runs while the delete
/// flow holds the engine-slot claim. A file that appears after this scan
/// is still hashed by [`remove_cached_model`] itself.
pub fn verified_snapshot_files(caches: &[PathBuf], spec: &ModelSpec) -> VerifiedSnapshotFiles {
	let mut verified = std::collections::HashMap::new();
	for cache in caches {
		let snapshots = hf_repo_dir(cache, spec).join("snapshots");
		let Ok(revs) = std::fs::read_dir(&snapshots) else {
			continue;
		};
		for rev in revs.flatten() {
			let target = rev.path().join(spec.filename);
			let Ok(meta) = target.symlink_metadata() else {
				continue;
			};
			if meta.file_type().is_symlink() || !meta.file_type().is_file() {
				continue;
			}
			let pinned = snapshot_file_is_pinned(&target, &meta, spec);
			verified.insert(target, pinned);
		}
	}
	verified
}

/// Remove this model's cache entry from its own catalog repo, kept
/// for the pinned content only - never a same-named file that holds
/// something else. A snapshot link goes when its target is
/// `blobs/<pinned sha>`; a regular file only after its size matches
/// the catalog and its streamed sha256 matches the pin; anything
/// unidentifiable (read errors, unresolvable links) is retained and
/// logged, so a delete cannot break another tool's revision (worst
/// case, a tool whose snapshot was intentionally kept re-downloads a
/// blob we removed as unreferenced). `verified` carries the hashes of
/// files already checked before the caller took its exclusive claim.
pub fn remove_cached_model(
	cache: &Path,
	spec: &ModelSpec,
	verified: &VerifiedSnapshotFiles,
) -> Result<bool, String> {
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
			// The pre-claim scan already hashed what it saw; only an
			// entry that appeared afterwards pays the hash here (the rare
			// path the pre-scan exists to avoid).
			let pinned = verified
				.get(&target)
				.copied()
				.flatten()
				.or_else(|| snapshot_file_is_pinned(&target, &meta, spec));
			match pinned {
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

#[cfg(test)]
mod hf_cache_tests {
	use super::hf_cache_model_path;
	use super::LLM_MODELS;

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
		use crate::models::ai_settings::resolve_hf_endpoint;
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
	use super::super::stream::part_path;
	use super::LLM_MODELS;
	use super::{
		hf_blob_path, hf_cache_model_path, materialize_snapshot, migrate_one, remove_cached_model,
		verified_snapshot_files,
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
	fn the_pre_scan_classifies_snapshot_files_before_the_claim() {
		let cache = tempfile::tempdir().expect("tempdir");
		let content = b"pinned model bytes";
		let spec = pinned_spec(content);
		let blob = hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, content).unwrap();
		// the app's own revision: a link on unix, which the pre-scan skips
		materialize_snapshot(cache.path(), &spec).expect("materialize");
		let snapshots = blob.parent().unwrap().parent().unwrap().join("snapshots");

		// a foreign regular file with the same name and different content
		let foreign = snapshots.join("foreign-rev");
		std::fs::create_dir_all(&foreign).unwrap();
		std::fs::write(foreign.join(spec.filename), b"someone else's").unwrap();
		// a regular file holding the pinned content
		let pinned_rev = snapshots.join("regular-pinned");
		std::fs::create_dir_all(&pinned_rev).unwrap();
		std::fs::write(pinned_rev.join(spec.filename), content).unwrap();

		let verified = verified_snapshot_files(&[cache.path().to_path_buf()], &spec);
		assert_eq!(verified.len(), 2, "symlinks are not hashed: {verified:?}");
		assert_eq!(
			verified.get(&foreign.join(spec.filename)),
			Some(&Some(false)),
			"foreign content is classified, not removed"
		);
		assert_eq!(
			verified.get(&pinned_rev.join(spec.filename)),
			Some(&Some(true)),
			"pinned content is pre-verified"
		);
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
		assert!(remove_cached_model(cache.path(), &spec, &Default::default()).expect("remove"));
		assert!(!hf_cache_model_path(cache.path(), &spec).is_some());
		assert!(
			blob.is_file(),
			"blob survives while another snapshot references it"
		);

		// the other tool's link is the only thing holding the blob now
		assert!(
			!remove_cached_model(cache.path(), &spec, &Default::default()).expect("no-op remove")
		);
		assert!(blob.is_file(), "still referenced: kept");
		// once that link is gone too, the next remove prunes the blob
		std::fs::remove_file(other.join("different-name.gguf")).unwrap();
		assert!(
			remove_cached_model(cache.path(), &spec, &Default::default()).expect("prune remove")
		);
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

		assert!(remove_cached_model(cache.path(), &spec, &Default::default()).expect("remove"));
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

		assert!(remove_cached_model(cache.path(), &spec, &Default::default()).expect("remove"));
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
		assert!(
			!remove_cached_model(cache.path(), &spec, &Default::default()).expect("no-op remove")
		);
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

		remove_cached_model(cache.path(), &spec, &Default::default()).expect("remove");
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
		let removed = remove_cached_model(cache.path(), &spec, &Default::default());
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
		assert!(remove_cached_model(cache.path(), &spec, &Default::default()).expect("remove"));

		let mut perms = std::fs::metadata(&locked).unwrap().permissions();
		perms.set_mode(0o755);
		std::fs::set_permissions(&locked, perms).unwrap();

		assert!(
			blob.is_file(),
			"an unreadable revision is uncertainty, not absence: the blob must not be pruned"
		);
	}
}
