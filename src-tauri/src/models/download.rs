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
/// hardlink (Windows without symlink privileges), then a copy. Idempotent.
pub fn materialize_snapshot(cache: &Path, spec: &ModelSpec) -> Result<(), String> {
	let snapshot_dir = hf_repo_dir(cache, spec).join("snapshots").join(spec.sha256);
	std::fs::create_dir_all(&snapshot_dir).map_err(|e| e.to_string())?;
	let link = snapshot_dir.join(spec.filename);
	if link.symlink_metadata().is_ok() {
		return Ok(());
	}
	let blob = hf_blob_path(cache, spec);
	if !blob.is_file() {
		return Err(format!("blob missing for {}", spec.id));
	}
	let rel = std::path::Path::new("../../blobs").join(spec.sha256);
	#[cfg(target_family = "unix")]
	{
		std::os::unix::fs::symlink(&rel, &link).map_err(|e| e.to_string())?;
	}
	#[cfg(target_os = "windows")]
	{
		use std::os::windows::fs as win_fs;
		if win_fs::symlink_file(&rel, &link).is_err() {
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
/// be renamed into a content-addressed store under a sha it doesn't
/// have. An existing blob means the content is already cached: the app
/// copy is redundant and simply removed.
pub fn migrate_legacy_models(models_dir: &Path, cache: &Path) {
	for spec in LLM_MODELS.iter().chain(STT_MODELS.iter()) {
		migrate_one(models_dir, cache, spec);
	}
}

/// Migrate a single spec's app-dir file into the cache (separate so
/// tests can drive it with fixture specs instead of the catalog pins).
fn migrate_one(models_dir: &Path, cache: &Path, spec: &ModelSpec) {
	let app_file = models_dir.join(spec.filename);
	if !app_file.is_file() {
		return;
	}
	let blob = hf_blob_path(cache, spec);
	if blob.is_file() {
		log::info!(
			"migrating {}: blob already cached, dropping the app copy",
			spec.id
		);
		if let Err(e) = std::fs::remove_file(&app_file) {
			log::warn!("could not remove the redundant app copy: {e}");
			return;
		}
	} else {
		match sha256_of_file(&app_file) {
			Some(hash) if hash.eq_ignore_ascii_case(spec.sha256) => {}
			other => {
				log::warn!(
					"leaving {} in the app models dir: its content does not match the pinned hash ({:?})",
					spec.id,
					other
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
		// rename within a volume; fall back to copy-via-.part across
		// volumes (a partial copy never lands under the final name)
		if std::fs::rename(&app_file, &blob).is_err() {
			let tmp = part_path(&blob);
			match std::fs::copy(&app_file, &tmp)
				.and_then(|_| std::fs::rename(&tmp, &blob))
				.and_then(|_| std::fs::remove_file(&app_file))
			{
				Ok(()) => {}
				Err(e) => {
					log::warn!("could not migrate {} into the cache: {e}", spec.id);
					let _ = std::fs::remove_file(&tmp);
					return;
				}
			}
		}
		log::info!("migrated {} into the hub cache", spec.id);
	}
	if let Err(e) = materialize_snapshot(cache, spec) {
		log::warn!("could not create the cache snapshot for {}: {e}", spec.id);
	}
}

/// True when some snapshot entry still links to `blobs/<sha>`.
/// Symlinks are inspected precisely; a non-symlink entry (hardlink or
/// copied fallback, e.g. on Windows) hides its target, so it is treated
/// as referencing the blob - never prune what might be in use.
fn blob_referenced(snapshots_dir: &Path, sha: &str) -> bool {
	for rev in std::fs::read_dir(snapshots_dir)
		.into_iter()
		.flatten()
		.flatten()
	{
		let rev_dir = rev.path();
		if !rev_dir.is_dir() {
			continue;
		}
		for entry in std::fs::read_dir(rev_dir).into_iter().flatten().flatten() {
			let path = entry.path();
			if !path.is_file() {
				continue; // broken symlink or directory
			}
			match std::fs::read_link(&path) {
				Ok(target) => {
					if target.file_name().map(|n| n == sha).unwrap_or(false) {
						return true;
					}
				}
				Err(_) => return true, // not a symlink: conservatively in use
			}
		}
	}
	false
}

/// Remove this model's cache entry: every `snapshots/*/<filename>` link,
/// then the blob when nothing else in the repo references it. This is
/// the same rule huggingface's own cache pruning applies, so deleting
/// in Brainstory never breaks another tool's snapshot (worst case, that
/// tool re-downloads a blob we removed as unreferenced).
pub fn remove_cached_model(cache: &Path, spec: &ModelSpec) -> Result<bool, String> {
	let repo_dir = hf_repo_dir(cache, spec);
	let snapshots = repo_dir.join("snapshots");
	if !snapshots.is_dir() {
		return Ok(false);
	}
	let mut removed = false;
	for rev in std::fs::read_dir(&snapshots)
		.map_err(|e| e.to_string())?
		.flatten()
	{
		let target = rev.path().join(spec.filename);
		if target.symlink_metadata().is_ok() {
			std::fs::remove_file(&target).map_err(|e| e.to_string())?;
			removed = true;
		}
	}
	let blob = hf_blob_path(cache, spec);
	if blob.is_file() && !blob_referenced(&snapshots, spec.sha256) {
		std::fs::remove_file(&blob).map_err(|e| e.to_string())?;
		removed = true;
	}
	Ok(removed)
}

/// The `.part` staging path for a download destination: `<file>.part`
/// appended to the full name (with_extension would collapse `x.bin` and
/// `x.gguf` to the same `x.part`).
fn part_path(dest: &Path) -> PathBuf {
	let mut name = dest.as_os_str().to_os_string();
	name.push(".part");
	PathBuf::from(name)
}

/// Stream a model file to disk, reporting progress through `on_progress`
/// (percentage 0-100). Verifies the download completed fully and matches
/// the pinned sha256 before moving it into place; the `.part` file is
/// removed on any failure.
pub async fn download_model_file(
	url: &str,
	dest: &Path,
	expected_size: u64,
	expected_sha256: &str,
	hf_token: &str,
	cancel: &AtomicBool,
	on_progress: &mut (impl FnMut(f64) + Send),
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
	let response = request
		.send()
		.await
		.map_err(|e| format!("download request failed: {e}"))?;
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

	// Hash the resumed prefix from disk so the integrity check still
	// covers the complete file, and pre-seed the byte counter.
	if resumed {
		if let Some(h) = hasher.as_mut() {
			let mut file = tokio::fs::File::open(&tmp)
				.await
				.map_err(|e| e.to_string())?;
			use tokio::io::AsyncReadExt;
			let mut buf = vec![0u8; 1024 * 1024];
			loop {
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

	let total = response.content_length().unwrap_or(0) + resume_from;
	// Fail fast when the advertised length already contradicts the spec:
	// streaming multi-GB only to reject it at the end wastes the transfer.
	if total > 0 && expected_size > 0 && total != expected_size {
		return Err(format!(
			"download size mismatch (server says {total} bytes, expected {expected_size}) - please retry"
		));
	}
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

	// Every failure path below removes the partial file, so a retry starts
	// clean instead of leaving gigabytes of junk behind.
	let outcome = async {
		// `downloaded` comes from the outer scope: it is pre-seeded with
		// the resumed prefix so totals and progress include it.
		let mut last_report: u64 = downloaded;
		const CHUNK_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
		loop {
			if cancel.load(Ordering::Relaxed) {
				return Err("download cancelled".into());
			}
			let chunk = match tokio::time::timeout(CHUNK_IDLE_TIMEOUT, stream.next()).await {
				Err(_) => return Err("download stalled (no data for 60s)".into()),
				Ok(Some(Ok(c))) => c,
				Ok(Some(Err(e))) => return Err(format!("download interrupted: {e}")),
				Ok(None) => break,
			};
			if let Some(hasher) = hasher.as_mut() {
				hasher.update(&chunk);
			}
			file.write_all(&chunk).await.map_err(|e| e.to_string())?;
			downloaded += chunk.len() as u64;
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
		file.flush().await.map_err(|e| e.to_string())?;
		// The stream can end "cleanly" mid-body; only a full-length file is
		// a valid model, anything else fails to load with cryptic errors.
		if total > 0 && downloaded != total {
			return Err(format!(
				"download incomplete (got {downloaded} of {total} bytes) - please retry"
			));
		}
		if expected_size > 0 && downloaded != expected_size {
			return Err(format!(
				"download size mismatch (got {downloaded} bytes, expected {expected_size}) - please retry"
			));
		}
		if let Some(hasher) = hasher.take() {
			let actual: String = hasher
				.finalize()
				.iter()
				.map(|b| format!("{b:02x}"))
				.collect();
			if !actual.eq_ignore_ascii_case(expected_sha256) {
				return Err(format!(
					"download failed its integrity check (sha256 {actual}) - the file was corrupted in transit or changed upstream; please retry"
				));
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
		Err(e) => {
			let _ = tokio::fs::remove_file(&tmp).await;
			Err(e)
		}
	}
}
#[cfg(test)]
mod download_tests {
	use super::{download_model_file, part_path};
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

	fn dest(tag: &str) -> std::path::PathBuf {
		part_path(
			&std::env::temp_dir().join(format!("brainstory-dl-cov-{tag}-{}", uuid::Uuid::new_v4())),
		)
		.with_file_name(format!(
			"brainstory-dl-cov-{tag}-{}.bin",
			uuid::Uuid::new_v4()
		))
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
		let dest = dest("auth");
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
		let dest = dest("anon");
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
		let dest = dest("indeterminate");
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
		let dest = dest("failfast");
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
				let (status, slice): (&str, &[u8]) = match range
					.as_deref()
					.and_then(|r| r.strip_prefix("bytes=").and_then(|r| r.split('-').next()))
					.and_then(|start| start.parse::<usize>().ok())
				{
					Some(start) if start < body.len() => ("206 Partial Content", &body[start..]),
					Some(_) => ("416 Range Not Satisfiable", &[]),
					None => ("200 OK", &body),
				};
				let head = format!(
					"HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n",
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
		let dest = part_path(
			&std::env::temp_dir().join(format!("brainstory-resume-{}.bin", uuid::Uuid::new_v4())),
		)
		.with_file_name(format!("brainstory-resume-{}.bin", uuid::Uuid::new_v4()));
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
		let _ = std::fs::remove_file(&dest);
	}

	#[tokio::test]
	async fn restarts_when_the_server_ignores_range() {
		let body = vec![9u8; 1500];
		let digest = sha256_hex(&body);
		let saw_range = std::sync::Arc::new(std::sync::Mutex::new(None));
		let url = serve_ranged(body.clone(), saw_range.clone());
		let dest = std::env::temp_dir().join(format!(
			"brainstory-resume-ign-{}.bin",
			uuid::Uuid::new_v4()
		));

		// stale prefix from a DIFFERENT transfer must not be stitched on
		std::fs::write(part_path(&dest), b"garbage prefix").expect("stage junk");

		let cancel = Arc::new(AtomicBool::new(false));
		// The server here honors Range, so make the junk prefix longer
		// than the body: the resume is refused and the download restarts.
		std::fs::write(part_path(&dest), vec![0u8; 2000]).expect("stage oversized junk");
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
		assert_eq!(std::fs::read(&dest).unwrap(), body);
		let _ = std::fs::remove_file(&dest);
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
		hf_blob_path, hf_cache_model_path, materialize_snapshot, migrate_one, remove_cached_model,
		LLM_MODELS,
	};
	use sha2::{Digest, Sha256};

	fn sha256_hex(bytes: &[u8]) -> String {
		Sha256::digest(bytes)
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect()
	}

	/// A spec-shaped fixture whose pinned sha matches `content`, so
	/// migration/materialization accept it.
	fn spec_for(content: &[u8]) -> super::ModelSpec {
		let mut spec = LLM_MODELS[0].clone();
		spec.sha256 = Box::leak(sha256_hex(content).into_boxed_str());
		spec
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
#[test]
fn part_paths_do_not_collide_across_extensions() {
	use std::path::Path;
	let bin = part_path(Path::new("/m/x.bin"));
	let gguf = part_path(Path::new("/m/x.gguf"));
	assert_eq!(bin, Path::new("/m/x.bin.part"));
	assert_eq!(gguf, Path::new("/m/x.gguf.part"));
	assert_ne!(bin, gguf, "staging names must be distinct");
}
