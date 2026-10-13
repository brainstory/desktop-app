//! The verified streaming download: resume-aware transfer with stall,
//! deadline and cancellation handling, plus staging-file hygiene.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::super::catalog::{ModelSpec, LLM_MODELS, STT_MODELS};
use super::USER_AGENT;

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
pub(super) fn part_path(dest: &Path) -> PathBuf {
	let mut name = dest.as_os_str().to_os_string();
	name.push(".part");
	PathBuf::from(name)
}

/// Remove a model's staged `.part` download, if one exists: the delete
/// flow calls this so a paused or interrupted transfer leaves nothing
/// behind.
pub fn remove_staged_part(base_file: &Path) -> Result<bool, String> {
	let part = part_path(base_file);
	if part.is_file() {
		std::fs::remove_file(&part).map_err(|e| e.to_string())?;
		return Ok(true);
	}
	Ok(false)
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

/// The exact error a user-initiated cancel produces. Callers that want to
/// distinguish "the user cancelled" from a real failure compare against
/// this (the UI shows no error for a cancel).
pub const DOWNLOAD_CANCELLED: &str = "download cancelled";

/// The error a user-paused download produces. Deliberately distinct from
/// a cancel: the staged `.part` survives, so resuming continues where the
/// transfer stopped instead of restarting from zero.
pub const DOWNLOAD_PAUSED: &str = "download paused";

/// For callers (and tests) that only ever cancel: a pause token that is
/// never set.
pub(super) static NEVER_PAUSED: AtomicBool = AtomicBool::new(false);

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
/// written. This entry never pauses; use
/// [`download_model_file_with_pause`] when the transfer should be
/// resumable on request.
pub async fn download_model_file(
	url: &str,
	dest: &Path,
	expected_size: u64,
	expected_sha256: &str,
	hf_token: &str,
	cancel: &AtomicBool,
	on_progress: &mut (impl FnMut(f64) + Send),
) -> Result<(), String> {
	download_model_file_with_pause(
		url,
		dest,
		expected_size,
		expected_sha256,
		hf_token,
		cancel,
		&NEVER_PAUSED,
		on_progress,
	)
	.await
}

/// [`download_model_file`] with a pause token: setting `pause` stops the
/// transfer keeping its staged prefix (the same policy as a transient
/// failure), so a later call resumes from it. Returns [`DOWNLOAD_PAUSED`]
/// as the error in that case.
#[allow(clippy::too_many_arguments)]
pub async fn download_model_file_with_pause(
	url: &str,
	dest: &Path,
	expected_size: u64,
	expected_sha256: &str,
	hf_token: &str,
	cancel: &AtomicBool,
	pause: &AtomicBool,
	on_progress: &mut (impl FnMut(f64) + Send),
) -> Result<(), String> {
	download_with_timeouts(
		url,
		dest,
		expected_size,
		expected_sha256,
		hf_token,
		cancel,
		pause,
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
	pause: &AtomicBool,
	on_progress: &mut (impl FnMut(f64) + Send),
	timeouts: Timeouts,
) -> Result<(), String> {
	use sha2::{Digest, Sha256};

	let tmp = part_path(dest);

	let client = reqwest::Client::builder()
		.connect_timeout(std::time::Duration::from_secs(15))
		// never let a redirect downgrade the token-bearing transfer to
		// plaintext http on a remote host
		.redirect(crate::models::hardened_redirect_policy())
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
			return Err(DOWNLOAD_CANCELLED.into());
		}
		if pause.load(Ordering::Relaxed) {
			// paused while waiting for the server: nothing was written
			// here, and any staged prefix stays for the resume
			return Err(DOWNLOAD_PAUSED.into());
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
					return Err(DOWNLOAD_CANCELLED.into());
				}
				if pause.load(Ordering::Relaxed) {
					drop(file);
					return Err(DOWNLOAD_PAUSED.into());
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
				return Err(Discard(DOWNLOAD_CANCELLED.into()));
			}
			if pause.load(Ordering::Relaxed) {
				// a pause keeps the staged prefix, exactly like a
				// transient failure: the next attempt resumes from it
				return Err(Resumable(DOWNLOAD_PAUSED.into()));
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
				&super::NEVER_PAUSED,
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
				&super::NEVER_PAUSED,
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

	/// Flip the pause flag from another thread after `delay`, the way the
	/// UI's pause button lands mid-download.
	fn pauser_after(delay: Duration, pause: &Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
		let pause = pause.clone();
		std::thread::spawn(move || {
			std::thread::sleep(delay);
			pause.store(true, std::sync::atomic::Ordering::Relaxed);
		})
	}

	#[tokio::test]
	async fn pausing_while_headers_are_pending_keeps_the_staged_prefix() {
		let url = serve_then_hold(Vec::new());
		let (dest, _dir) = dest("pausehdr");
		let tmp = part_path(&dest);
		std::fs::write(&tmp, b"abc").expect("stage prefix");
		let cancel = Arc::new(AtomicBool::new(false));
		let pause = Arc::new(AtomicBool::new(false));
		let pauser = pauser_after(Duration::from_millis(100), &pause);
		let err = tokio::time::timeout(
			Duration::from_secs(5),
			download_with_timeouts(
				&url,
				&dest,
				6,
				&sha256_hex(b"abcdef"),
				"",
				&cancel,
				&pause,
				&mut |_| {},
				test_timeouts(10_000, Duration::from_secs(5)),
			),
		)
		.await
		.expect("pause must not wait out the header deadline")
		.expect_err("a paused download must fail with its own message");
		assert_eq!(err, super::DOWNLOAD_PAUSED);
		// unlike a cancel, a pause keeps everything: the resume needs it
		assert_eq!(std::fs::read(&tmp).expect("prefix kept"), b"abc".to_vec());
		assert!(!dest.exists(), "dest is never touched on error");
		pauser.join().unwrap();
	}

	#[tokio::test]
	async fn pausing_during_a_stalled_body_keeps_the_staged_bytes() {
		// headers plus a partial body, then the server goes silent: the
		// pause must land within the short poll bound and keep the prefix
		let url = serve_then_hold(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nprefix".to_vec());
		let (dest, _dir) = dest("pausebody");
		let cancel = Arc::new(AtomicBool::new(false));
		let pause = Arc::new(AtomicBool::new(false));
		let pauser = pauser_after(Duration::from_millis(150), &pause);
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
				&pause,
				&mut |_| {},
				test_timeouts(5_000, Duration::from_secs(10)),
			),
		)
		.await
		.expect("pause must not wait out the stall timeout")
		.expect_err("a paused download must fail with its own message");
		assert_eq!(err, super::DOWNLOAD_PAUSED);
		assert!(
			start.elapsed() < Duration::from_secs(1),
			"pause answered in {:?}, not after the stall timeout",
			start.elapsed()
		);
		// the received bytes stay staged, ready for a Range resume
		assert_eq!(
			std::fs::read(part_path(&dest)).expect("partial body kept"),
			b"prefix".to_vec()
		);
		assert!(!dest.exists());
		pauser.join().unwrap();
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
				&super::NEVER_PAUSED,
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
				&super::NEVER_PAUSED,
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
				&super::NEVER_PAUSED,
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
	use super::LLM_MODELS;
	use super::{sweep_stale_part_files, PART_MAX_AGE};

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
