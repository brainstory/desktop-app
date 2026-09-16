//! Microphone capture via cpal. The WKWebView getUserMedia path delivers
//! silent audio in some TCC/permission states, so recording happens in the
//! Rust process (which holds the app's macOS microphone permission).

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

struct Capture {
	_stream: cpal::Stream,
	sample_rate: u32,
	channels: u16,
	samples: Arc<Mutex<Vec<f32>>>,
}

static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);

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
		sample_rate,
		channels: channel_count,
		samples,
	});
	Ok(())
}

/// Stop capturing and return the recording as a 16 kHz mono WAV file.
/// Dropping the cpal stream stops the device.
pub fn stop_capture() -> Result<Vec<u8>, String> {
	let mut guard = CAPTURE.lock().unwrap_or_else(|e| e.into_inner());
	let capture = guard.take().ok_or_else(|| "not recording".to_string())?;
	drop(capture._stream);

	let samples = capture
		.samples
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.clone();
	drop(guard);
	if samples.is_empty() {
		return Err("no audio captured".into());
	}

	// fold interleaved channels to mono, then resample to 16 kHz
	let channels = capture.channels.max(1) as usize;
	let mono: Vec<f32> = if channels > 1 {
		samples
			.chunks(channels)
			.map(|frame| frame.iter().sum::<f32>() / channels as f32)
			.collect()
	} else {
		samples
	};
	let mono = crate::stt::resample_to_16k(mono, capture.sample_rate)?;

	encode_wav_16k(&mono)
}

fn encode_wav_16k(samples: &[f32]) -> Result<Vec<u8>, String> {
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
