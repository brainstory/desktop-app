//! Microphone capture via cpal. The WKWebView getUserMedia path delivers
//! silent audio in some TCC/permission states, so recording happens in the
//! Rust process (which holds the app's macOS microphone permission).

use std::sync::{
	atomic::{AtomicU64, Ordering},
	Arc, Mutex,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Idle / stale stop sentinel. The frontend hook matches this exact
/// string to classify a stop as already-released (idempotent, not a
/// user-facing failure); a typed IPC error enum was considered and
/// deferred, so the string is the contract (pinned by tests below).
const NOT_RECORDING: &str = "not recording";
/// Empty-buffer sentinel returned after the stream was already dropped
/// (the device is already released when this surfaces).
const NO_AUDIO_CAPTURED: &str = "no audio captured";

struct Capture {
	_stream: cpal::Stream,
	/// the page generation (see PAGE_GENERATION) that started this
	/// capture: only that generation may stop it and receive the WAV
	generation: u64,
	sample_rate: u32,
	channels: u16,
	samples: Arc<Mutex<Vec<f32>>>,
}

static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);

/// Generation of the main webview's current page. Every page
/// abandonment bumps it; a capture remembers the generation it started
/// under, so cleanup belonging to a dead page can neither transcribe
/// abandoned audio nor stop a newer page's capture.
static PAGE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// What a stop request means for the active capture. Pure decision
/// over generations (no audio hardware) so the ownership rules are
/// unit-testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopDecision {
	/// the caller still owns the capture: stop it and convert to WAV
	Proceed,
	/// idle, or a capture belonging to another generation: report the
	/// existing "not recording" error; a newer capture stays untouched
	NotRecording,
}

/// May a stop issued by `caller_generation` take the capture started
/// by `capture_generation` (None = idle)? The live stop path always
/// passes the CURRENT page generation as the caller (per-stop IPC
/// tokens were considered and deferred); the unit tests also pass
/// stale generations to prove a dead page's late stop cannot take a
/// capture started by a newer page.
fn resolve_stop(capture_generation: Option<u64>, caller_generation: u64) -> StopDecision {
	match capture_generation {
		Some(g) if g == caller_generation => StopDecision::Proceed,
		_ => StopDecision::NotRecording,
	}
}

/// Append converted samples to the capture buffer unless the length cap is
/// hit. Runs in the audio callback: lock briefly, no heavy work.
fn queue_samples(
	queue: &Arc<Mutex<Vec<f32>>>,
	max_samples: usize,
	samples: impl Iterator<Item = f32>,
) {
	let mut buf = queue.lock().unwrap_or_else(|e| e.into_inner());
	if buf.len() < max_samples {
		buf.extend(samples);
	}
}

/// Hard cap on capture length (a little above the UI's 4-minute recording
/// limit) so an abandoned recording can't grow the buffer unboundedly.
const MAX_CAPTURE_SECS: usize = 300;

/// Begin capturing from the default input device. Errors if already running
/// or if macOS microphone permission has not been granted.
pub fn start_capture() -> Result<(), String> {
	let mut guard = CAPTURE.lock().unwrap_or_else(|e| e.into_inner());
	// Read under the CAPTURE lock so this cannot race abandon_capture
	// (which empties the slot and bumps the generation under the same
	// lock): the capture is filed under the page generation that
	// actually owns it.
	let generation = PAGE_GENERATION.load(Ordering::SeqCst);
	if guard.is_some() {
		// Already recording - treat as success so a double-press of the
		// record button (first press still starting the stream) is harmless.
		return Ok(());
	}

	let host = cpal::default_host();
	let device = host
		.default_input_device()
		.ok_or_else(|| "no microphone found".to_string())?;

	let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
	let queue = Arc::clone(&samples);

	// Try the default config first; some devices reject it with obscure
	// coreaudio errors, so fall back through every supported config.
	let mut build_error: Option<String> = None;
	let mut built: Option<(cpal::Stream, u32, u16)> = None;
	let candidates: Vec<(cpal::SampleFormat, cpal::StreamConfig)> =
		match device.default_input_config() {
			Ok(default) => {
				let mut v = vec![(default.sample_format(), default.config())];
				if let Ok(all) = device.supported_input_configs() {
					v.extend(all.filter_map(|range| {
						let cfg = range.try_with_sample_rate(16_000)?;
						Some((cfg.sample_format(), cfg.config()))
					}));
				}
				v
			}
			Err(e) => {
				build_error = Some(format!("failed to read microphone config: {e}"));
				device
					.supported_input_configs()
					.map(|all| {
						all.filter_map(|range| {
							let cfg = range.try_with_sample_rate(16_000)?;
							Some((cfg.sample_format(), cfg.config()))
						})
						.collect()
					})
					.unwrap_or_default()
			}
		};

	for (sample_format, stream_config) in &candidates {
		let queue = Arc::clone(&queue);
		// Stop appending past the cap; a stream left running by a bug or a
		// crashed UI must not eat memory forever.
		let max_samples =
			stream_config.sample_rate as usize * stream_config.channels as usize * MAX_CAPTURE_SECS;
		let err_fn = move |err| log::warn!("microphone stream error: {err}");
		// F32 is the native whisper input; I16/U16 devices are common on
		// other platforms, and converting is trivial - so accept them
		// instead of reporting "microphone unsupported".
		let stream = match sample_format {
			cpal::SampleFormat::F32 => device.build_input_stream(
				*stream_config,
				move |data: &[f32], _: &cpal::InputCallbackInfo| {
					queue_samples(&queue, max_samples, data.iter().copied());
				},
				err_fn,
				None,
			),
			cpal::SampleFormat::I16 => device.build_input_stream(
				*stream_config,
				move |data: &[i16], _: &cpal::InputCallbackInfo| {
					queue_samples(
						&queue,
						max_samples,
						data.iter().map(|&s| s as f32 / 32768.0),
					);
				},
				err_fn,
				None,
			),
			cpal::SampleFormat::U16 => device.build_input_stream(
				*stream_config,
				move |data: &[u16], _: &cpal::InputCallbackInfo| {
					queue_samples(
						&queue,
						max_samples,
						data.iter().map(|&s| (s as f32 - 32768.0) / 32768.0),
					);
				},
				err_fn,
				None,
			),
			_ => continue,
		};
		match stream {
			Ok(stream) => {
				if let Err(e) = stream.play() {
					build_error = Some(format!("failed to start microphone: {e}"));
					continue;
				}
				built = Some((stream, stream_config.sample_rate, stream_config.channels));
				break;
			}
			Err(e) => {
				build_error = Some(format!("failed to open microphone: {e}"));
			}
		}
	}

	let (stream, sample_rate, channel_count) =
		built.ok_or_else(|| build_error.unwrap_or_else(|| "microphone unsupported".into()))?;
	// The stream is already playing from the candidate loop above.

	*guard = Some(Capture {
		_stream: stream,
		generation,
		sample_rate,
		channels: channel_count,
		samples,
	});
	Ok(())
}

/// The page that owns the microphone is going away (new page load,
/// window destroyed, app exit): stop any active capture and DISCARD
/// the audio - no WAV conversion, it must never be transcribed - and
/// bump the page generation so a late stop from the dead page reads
/// "not recording" instead of stealing a newer capture. Idempotent:
/// no error when idle (every ordinary page load lands here).
pub fn abandon_capture() {
	let mut guard = CAPTURE.lock().unwrap_or_else(|e| e.into_inner());
	// dropping the cpal stream stops the device; the samples are dropped
	*guard = None;
	PAGE_GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// Stop capturing and return the recording as a 16 kHz mono WAV file.
/// Dropping the cpal stream stops the device. Only a capture belonging
/// to the CURRENT page generation may be stopped: audio from an
/// abandoned page is never converted, and a capture started by a newer
/// page is never touched by a stop that was issued under an older one.
pub fn stop_capture() -> Result<Vec<u8>, String> {
	// Read the caller's generation BEFORE taking the CAPTURE lock: a
	// stop racing a page abandonment then carries the generation it
	// was issued under, so it cannot take a capture the new page
	// started in the meantime.
	let caller_generation = PAGE_GENERATION.load(Ordering::SeqCst);
	let mut guard = CAPTURE.lock().unwrap_or_else(|e| e.into_inner());
	let capture_generation = guard.as_ref().map(|capture| capture.generation);
	if resolve_stop(capture_generation, caller_generation) == StopDecision::NotRecording {
		if matches!(capture_generation, Some(g) if g < caller_generation) {
			// Unreachable while abandon_capture empties the slot under
			// this same lock before the generation can move past a
			// capture - but if a stale capture ever survives anyway,
			// discard it here too: its audio must never be transcribed.
			// A capture from a NEWER generation is never touched.
			*guard = None;
		}
		return Err(NOT_RECORDING.to_string());
	}
	let capture = guard
		.take()
		.expect("resolve_stop only proceeds with an active capture");
	drop(capture._stream);

	// take (not clone): the capture is gone after this call anyway, and
	// the buffer can hold ~minutes of audio
	let mut samples_guard = capture.samples.lock().unwrap_or_else(|e| e.into_inner());
	let samples = std::mem::take(&mut *samples_guard);
	drop(samples_guard);
	drop(guard);
	capture_to_wav(samples, capture.channels, capture.sample_rate)
}

/// Turn the raw interleaved capture buffer into a 16 kHz mono WAV: fold
/// the channels to mono, resample, encode.
fn capture_to_wav(samples: Vec<f32>, channels: u16, sample_rate: u32) -> Result<Vec<u8>, String> {
	if samples.is_empty() {
		return Err(NO_AUDIO_CAPTURED.to_string());
	}
	let channels = channels.max(1) as usize;
	// Defense in depth: the device-side buffer is already capped at
	// MAX_CAPTURE_SECS, but the conversion validates the same budget
	// (rate range + duration bound, checked arithmetic) before folding
	// and resampling, so a bug in the cap cannot turn into an unbounded
	// allocation here.
	let frames = samples.len().div_ceil(channels);
	crate::stt::checked_resample_len(frames, sample_rate)?;
	let mono = crate::stt::fold_to_mono(samples, channels);
	let mono = crate::stt::resample_to_16k(mono, sample_rate)?;
	encode_wav_16k(&mono)
}

pub(crate) fn encode_wav_16k(samples: &[f32]) -> Result<Vec<u8>, String> {
	let data_len = (samples.len() * 2) as u32;
	let mut out: Vec<u8> = Vec::with_capacity(44 + samples.len() * 2);
	out.extend_from_slice(b"RIFF");
	out.extend_from_slice(&(36 + data_len).to_le_bytes());
	out.extend_from_slice(b"WAVE");
	out.extend_from_slice(b"fmt ");
	out.extend_from_slice(&16u32.to_le_bytes());
	out.extend_from_slice(&1u16.to_le_bytes()); // PCM
	out.extend_from_slice(&1u16.to_le_bytes()); // mono
	out.extend_from_slice(&16_000u32.to_le_bytes());
	out.extend_from_slice(&32_000u32.to_le_bytes()); // byte rate
	out.extend_from_slice(&2u16.to_le_bytes()); // block align
	out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
	out.extend_from_slice(b"data");
	out.extend_from_slice(&data_len.to_le_bytes());
	for &sample in samples {
		let clamped = sample.clamp(-1.0, 1.0);
		out.extend_from_slice(&((clamped * 32767.0) as i16).to_le_bytes());
	}
	Ok(out)
}

#[cfg(test)]
mod wav_tests {
	#[test]
	fn capture_folds_stereo_including_a_truncated_final_frame() {
		// L/R pairs, then a lone left sample: a capture stopped mid-frame
		let interleaved = vec![0.5, -0.5, 0.2, 0.4, 0.6];
		let wav = super::capture_to_wav(interleaved, 2, 16_000).expect("encode");
		let mono = crate::stt::wav_to_samples(&wav).expect("decode");
		assert_eq!(mono.len(), 3, "one sample per (possibly partial) frame");
		assert!(mono[0].abs() < 1e-3, "opposite channels cancel");
		assert!((mono[1] - 0.3).abs() < 1e-3, "frames are averaged");
		// the partial frame is its own average, not 0.6 / 2
		assert!((mono[2] - 0.6).abs() < 1e-3, "got {}", mono[2]);
	}

	#[test]
	fn capture_rejects_an_empty_buffer() {
		assert!(super::capture_to_wav(Vec::new(), 1, 48_000).is_err());
	}

	#[test]
	fn capture_conversion_rejects_audio_over_the_300s_bound() {
		// the device-side buffer is capped at MAX_CAPTURE_SECS, but the
		// conversion validates the same budget: 300_001 frames at 1 kHz
		// is one sample past the five-minute bound
		match super::capture_to_wav(vec![0.0; 300_001], 1, 1_000) {
			Err(e) => assert!(e.contains("too long"), "{e}"),
			Ok(wav) => panic!(
				"over-bound capture accepted: {} byte WAV from 300_001 samples at 1 kHz",
				wav.len()
			),
		}
	}

	#[test]
	fn capture_converts_a_short_48k_recording() {
		let wav = super::capture_to_wav(vec![0.25; 48], 1, 48_000).expect("convert");
		let mono = crate::stt::wav_to_samples(&wav).expect("decode");
		assert_eq!(mono.len(), 16, "48 samples at 48 kHz -> 16 at 16 kHz");
		assert!((mono[0] - 0.25).abs() < 1e-3, "{}", mono[0]);
	}

	#[test]
	fn capture_cap_and_conversion_bound_stay_pinned_together() {
		// the device-side buffer cap and the conversion budget are two
		// spellings of one limit: drift would let one path accept what
		// the other rejects
		assert_eq!(super::MAX_CAPTURE_SECS, crate::stt::MAX_AUDIO_SECS);
	}

	#[test]
	fn encode_wav_16k_roundtrips_through_wav_to_samples() {
		let samples: Vec<f32> = vec![0.0, 0.5, -0.5, 0.99, -1.0];
		let wav = super::encode_wav_16k(&samples).expect("encode");
		let decoded = crate::stt::wav_to_samples(&wav).expect("decode");
		assert_eq!(decoded.len(), samples.len(), "same sample count");
		for (original, roundtripped) in samples.iter().zip(decoded.iter()) {
			assert!(
				(original - roundtripped).abs() < 0.001,
				"{original} vs {roundtripped}"
			);
		}
	}
}

/// Page-generation ownership of the microphone: hardware-free tests.
/// The live statics (CAPTURE, PAGE_GENERATION) are process-global and
/// tests run in parallel threads, so the ones touching them serialize
/// on LIVE; the pure resolve_stop tests never touch hardware or
/// statics.
#[cfg(test)]
mod generation_tests {
	use std::sync::atomic::Ordering;

	use super::{abandon_capture, resolve_stop, stop_capture, StopDecision, PAGE_GENERATION};

	static LIVE: std::sync::Mutex<()> = std::sync::Mutex::new(());

	#[test]
	fn abandon_bumps_the_generation_and_a_late_stop_is_not_recording() {
		let _live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
		let before = PAGE_GENERATION.load(Ordering::SeqCst);
		// abandoning while idle must not error: every ordinary page
		// load, window destroy and app exit calls this
		abandon_capture();
		abandon_capture();
		assert_eq!(
			PAGE_GENERATION.load(Ordering::SeqCst),
			before + 2,
			"every abandonment starts a new page generation"
		);
		// a stop landing after its page went away reads as the existing
		// already-stopped error, not a new failure mode
		assert_eq!(stop_capture().unwrap_err(), "not recording");
	}

	#[test]
	fn the_generation_that_started_a_capture_may_stop_it() {
		assert_eq!(resolve_stop(Some(3), 3), StopDecision::Proceed);
		// idle: "not recording" for any caller
		assert_eq!(resolve_stop(None, 3), StopDecision::NotRecording);
	}

	#[test]
	fn a_stale_generation_stop_never_takes_a_newer_capture() {
		// page 1 (generation 0) records; the navigation abandons its
		// audio and bumps the generation (tested live above); page 2
		// starts its own capture under generation 1
		let page1 = 0;
		let page2 = page1 + 1;
		let page2_capture = Some(page2);
		// page 1's late stop: "not recording", and the newer capture is
		// untouched - its owner can still stop it normally afterwards
		assert_eq!(
			resolve_stop(page2_capture, page1),
			StopDecision::NotRecording
		);
		assert_eq!(resolve_stop(page2_capture, page2), StopDecision::Proceed);
	}

	#[test]
	fn frontend_matched_error_strings_stay_exact() {
		// the frontend hook classifies stop errors by these exact
		// strings (a typed IPC error enum was considered and deferred):
		// rewording one silently breaks its return-to-idle path
		assert_eq!(super::NOT_RECORDING, "not recording");
		assert_eq!(super::NO_AUDIO_CAPTURED, "no audio captured");
	}
}
