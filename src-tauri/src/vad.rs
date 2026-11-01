//! Voice-activity detection: Silero VAD (whisper.cpp's ggml port) finds
//! the stretches of a recording that contain speech before any speech
//! model runs.
//!
//! Two jobs depend on it:
//!
//! - **Whisper** invents text over silence and room noise ("Thank you
//!   for watching!"). Feeding it only the speech stretches removes the
//!   input it hallucinates on, and skips decoding the pauses.
//! - **Parakeet** (an ONNX model with full attention) is run per chunk
//!   of at most [`MAX_CHUNK_SECS`]; chunks are cut at the pauses VAD
//!   reports instead of mid-word.
//!
//! The model is ~0.9 MB (MIT, snakers4/silero-vad, converted by
//! ggml-org/whisper-vad) and ships inside the binary: there is nothing
//! to download, and every install has it. whisper.cpp loads models from
//! a path, so the embedded bytes are written into the models directory
//! on first use.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// ggml-org/whisper-vad `ggml-silero-v6.2.0.bin`, sha256
/// 2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987.
const MODEL_BYTES: &[u8] = include_bytes!("../resources/vad/ggml-silero-v6.2.0.bin");
const MODEL_FILE: &str = "ggml-silero-v6.2.0.bin";

/// Sample rate VAD and every local speech model work at.
const RATE: usize = 16_000;

/// Speech probability above which a frame counts as speech. Below
/// Silero's 0.5 default: dropping a quiet sentence loses words for good,
/// while letting a little noise through only costs the speech model a
/// moment (and whisper's own no-speech filters still apply).
const THRESHOLD: f32 = 0.4;
/// Speech shorter than this is a click or a cough, not a word.
const MIN_SPEECH_MS: i32 = 200;
/// A pause must last this long to end a speech segment; shorter gaps
/// (between words, a breath) stay inside the segment.
const MIN_SILENCE_MS: i32 = 300;
/// Audio kept on each side of a segment so word onsets and trailing
/// consonants are not clipped.
const SPEECH_PAD_MS: i32 = 200;
/// Longest segment VAD returns; longer uninterrupted speech is split at
/// its quietest point, keeping every segment below [`MAX_CHUNK_SECS`].
const MAX_SEGMENT_SECS: f32 = 20.0;

/// Longest chunk handed to a chunked speech model (Parakeet). The model
/// attends over the whole chunk, so memory grows with its square;
/// 30 s keeps that small while giving the model whole sentences.
pub const MAX_CHUNK_SECS: usize = 30;

/// Silence inserted between joined speech segments, so the speech model
/// still hears a sentence boundary where a pause was cut out.
const JOIN_GAP_MS: usize = 150;

static VAD: OnceLock<Result<Mutex<whisper_rs::WhisperVadContext>, String>> = OnceLock::new();

/// Write the embedded model into `models_dir` (unless an identical copy
/// is already there) and return its path. Written to a temporary name
/// and renamed into place, so a crash can never leave a truncated model
/// for the next launch to load.
fn materialize_model(models_dir: &Path) -> Result<PathBuf, String> {
	let dir = models_dir.join("vad");
	let path = dir.join(MODEL_FILE);
	if std::fs::read(&path).is_ok_and(|bytes| bytes == MODEL_BYTES) {
		return Ok(path);
	}
	std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
	let tmp = dir.join(format!("{MODEL_FILE}.tmp"));
	std::fs::write(&tmp, MODEL_BYTES).map_err(|e| format!("write {}: {e}", tmp.display()))?;
	std::fs::rename(&tmp, &path).map_err(|e| format!("install {}: {e}", path.display()))?;
	Ok(path)
}

fn load(models_dir: &Path) -> Result<Mutex<whisper_rs::WhisperVadContext>, String> {
	let path = materialize_model(models_dir)?;
	let path = path
		.to_str()
		.ok_or_else(|| format!("VAD model path is not UTF-8: {}", path.display()))?;
	let mut params = whisper_rs::WhisperVadContextParams::new();
	params.set_n_threads(2);
	// a sub-megabyte LSTM: the CPU is faster than a GPU round-trip
	params.set_use_gpu(false);
	whisper_rs::WhisperVadContext::new(path, params)
		.map(Mutex::new)
		.map_err(|e| format!("failed to load the VAD model: {e}"))
}

/// The sample ranges of `samples` (16 kHz mono) that contain speech,
/// padded and in order. `Err` means VAD itself is unavailable (the
/// model could not be written or loaded); callers then fall back to
/// the whole recording rather than losing it. An empty list is a real
/// answer: nobody spoke.
pub fn speech_ranges(models_dir: &Path, samples: &[f32]) -> Result<Vec<Range<usize>>, String> {
	let vad = VAD
		.get_or_init(|| load(models_dir))
		.as_ref()
		.map_err(Clone::clone)?;
	let mut params = whisper_rs::WhisperVadParams::new();
	params.set_threshold(THRESHOLD);
	params.set_min_speech_duration(MIN_SPEECH_MS);
	params.set_min_silence_duration(MIN_SILENCE_MS);
	params.set_max_speech_duration(MAX_SEGMENT_SECS);
	params.set_speech_pad(SPEECH_PAD_MS);
	let mut ctx = vad.lock().unwrap_or_else(|e| e.into_inner());
	let segments = ctx
		.segments_from_samples(params, samples)
		.map_err(|e| format!("voice activity detection failed: {e}"))?;
	// whisper.cpp reports segment bounds in centiseconds
	let centis: Vec<(f32, f32)> = segments.map(|s| (s.start, s.end)).collect();
	Ok(centis_to_ranges(&centis, samples.len()))
}

/// Convert centisecond segment bounds to sample ranges, clamped to the
/// recording and with overlaps (from padding) merged.
fn centis_to_ranges(centis: &[(f32, f32)], len: usize) -> Vec<Range<usize>> {
	let per_cs = RATE as f32 / 100.0;
	let mut ranges: Vec<Range<usize>> = Vec::new();
	for &(start, end) in centis {
		let start = ((start.max(0.0) * per_cs) as usize).min(len);
		let end = ((end.max(0.0) * per_cs).ceil() as usize).min(len);
		if end <= start {
			continue;
		}
		match ranges.last_mut() {
			Some(last) if start <= last.end => last.end = last.end.max(end),
			_ => ranges.push(start..end),
		}
	}
	ranges
}

/// The speech in `samples`, with the pauses between `ranges` shortened
/// to [`JOIN_GAP_MS`] of silence.
pub fn join_speech(samples: &[f32], ranges: &[Range<usize>]) -> Vec<f32> {
	let gap = RATE * JOIN_GAP_MS / 1000;
	let mut out = Vec::with_capacity(ranges.iter().map(|r| r.len() + gap).sum());
	for (i, range) in ranges.iter().enumerate() {
		if i > 0 {
			out.resize(out.len() + gap, 0.0);
		}
		out.extend_from_slice(&samples[range.clone()]);
	}
	out
}

/// Group speech `ranges` into chunks of at most [`MAX_CHUNK_SECS`]
/// each, cutting only at the pauses between ranges. Each chunk is the
/// joined speech of its ranges (see [`join_speech`]). A single range
/// longer than the limit (VAD caps segments at [`MAX_SEGMENT_SECS`], so
/// only when VAD is bypassed) is split at fixed points.
pub fn chunk_speech(samples: &[f32], ranges: &[Range<usize>]) -> Vec<Vec<f32>> {
	let max = MAX_CHUNK_SECS * RATE;
	let mut chunks: Vec<Vec<f32>> = Vec::new();
	let mut group: Vec<Range<usize>> = Vec::new();
	let mut group_len = 0usize;
	let gap = RATE * JOIN_GAP_MS / 1000;
	for range in ranges {
		// an over-long range is cut into pieces of at most `max`
		let mut start = range.start;
		while start < range.end {
			let piece = start..range.end.min(start + max);
			start = piece.end;
			let added = piece.len() + if group.is_empty() { 0 } else { gap };
			if !group.is_empty() && group_len + added > max {
				chunks.push(join_speech(samples, &group));
				group.clear();
				group_len = 0;
			}
			group_len += piece.len() + if group.is_empty() { 0 } else { gap };
			group.push(piece);
		}
	}
	if !group.is_empty() {
		chunks.push(join_speech(samples, &group));
	}
	chunks
}

#[cfg(test)]
mod tests {
	use super::{centis_to_ranges, chunk_speech, join_speech, speech_ranges, MAX_CHUNK_SECS, RATE};

	#[test]
	fn centiseconds_become_merged_sample_ranges() {
		// 0.5-1.0 s, an overlapping 0.9-1.5 s (padding), then 3-4 s
		let ranges = centis_to_ranges(&[(50.0, 100.0), (90.0, 150.0), (300.0, 400.0)], 10 * RATE);
		assert_eq!(ranges, vec![8_000..24_000, 48_000..64_000]);
		// bounds past the end of the audio are clamped, empty ones dropped
		assert_eq!(
			centis_to_ranges(&[(50.0, 900.0), (950.0, 960.0)], RATE),
			vec![8_000..RATE]
		);
	}

	#[test]
	fn joined_speech_keeps_a_short_gap_between_segments() {
		let samples: Vec<f32> = (0..10).map(|i| i as f32).collect();
		let joined = join_speech(&samples, &[1..3, 7..9]);
		let gap = RATE * super::JOIN_GAP_MS / 1000;
		assert_eq!(joined.len(), 4 + gap);
		assert_eq!(&joined[..2], &[1.0, 2.0]);
		assert!(joined[2..2 + gap].iter().all(|s| *s == 0.0));
		assert_eq!(&joined[2 + gap..], &[7.0, 8.0]);
	}

	#[test]
	fn chunks_cut_at_pauses_and_respect_the_limit() {
		let max = MAX_CHUNK_SECS * RATE;
		let samples = vec![0.1f32; 3 * max];
		// three 12 s segments: two fit one chunk, the third starts the next
		let seg = 12 * RATE;
		let ranges = [
			0..seg,
			seg + RATE..2 * seg + RATE,
			2 * seg + 2 * RATE..3 * seg + 2 * RATE,
		];
		let chunks = chunk_speech(&samples, &ranges);
		assert_eq!(chunks.len(), 2);
		assert!(chunks.iter().all(|c| c.len() <= max));
		// one unbroken 70 s range is split into pieces of at most 30 s
		let chunks = chunk_speech(&samples, std::slice::from_ref(&(0..70 * RATE)));
		assert_eq!(chunks.iter().map(Vec::len).sum::<usize>(), 70 * RATE);
		assert!(
			chunks.iter().all(|c| c.len() <= max),
			"{:?}",
			chunks.iter().map(Vec::len).collect::<Vec<_>>()
		);
		assert!(chunk_speech(&samples, &[]).is_empty());
	}

	#[test]
	fn the_embedded_model_loads_and_hears_no_speech_in_silence() {
		let dir = tempfile::tempdir().expect("tempdir");
		let silence = vec![0.0f32; 3 * RATE];
		let ranges = speech_ranges(dir.path(), &silence).expect("VAD runs");
		assert!(
			ranges.is_empty(),
			"speech found in digital silence: {ranges:?}"
		);
		assert!(dir.path().join("vad").join(super::MODEL_FILE).is_file());
	}
}
