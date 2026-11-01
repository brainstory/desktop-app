//! Optional noise suppression (RNNoise, via the pure-Rust `nnnoiseless`
//! port) applied to a recording before it is transcribed.
//!
//! Off by default (Settings → AI Models → "Reduce background noise"):
//! Whisper and Parakeet are trained on noisy audio, and suppression can
//! smear consonants they would otherwise have heard, so it pays off in
//! loud rooms (fans, traffic, a café) and can cost a little accuracy in
//! quiet ones. RNNoise is a ~85 KB recurrent network compiled into the
//! binary: no download, no extra runtime, and roughly real-time × 100
//! on one laptop core.

use nnnoiseless::DenoiseState;

/// The rate RNNoise is trained on and operates at.
const RNNOISE_RATE: u32 = 48_000;
/// The rate every recording is stored and transcribed at.
const SPEECH_RATE: u32 = 16_000;
/// RNNoise works on f32 samples in i16 range, not [-1, 1].
const I16_SCALE: f32 = 32_768.0;

/// Suppress background noise in 16 kHz mono `samples`, returning the
/// same number of samples. The audio is band-limited upsampled to
/// 48 kHz (the 16 kHz recording has nothing above 8 kHz, which RNNoise
/// handles as silent bands), denoised frame by frame, and resampled
/// back.
pub fn denoise_16k(samples: &[f32]) -> Vec<f32> {
	if samples.is_empty() {
		return Vec::new();
	}
	let factor = (RNNOISE_RATE / SPEECH_RATE) as usize;
	let up = crate::stt::resample_sinc(samples, SPEECH_RATE, RNNOISE_RATE, samples.len() * factor);
	let cleaned = denoise_48k(&up);
	crate::stt::resample_sinc(&cleaned, RNNOISE_RATE, SPEECH_RATE, samples.len())
}

/// Run RNNoise over 48 kHz mono `input`, returning the same length,
/// time-aligned with the input.
///
/// RNNoise analyses a two-frame window and overlap-adds its output, so
/// each output frame is the denoised *previous* input frame: the output
/// lags by exactly one frame. One frame of trailing silence is fed in to
/// flush the last real frame out, and the first output frame (the
/// lag) is dropped.
fn denoise_48k(input: &[f32]) -> Vec<f32> {
	let frame = DenoiseState::FRAME_SIZE;
	let mut state = DenoiseState::new();
	let padded_len = input.len().div_ceil(frame) * frame + frame;
	let mut out = Vec::with_capacity(padded_len);
	let mut in_buf = vec![0.0f32; frame];
	let mut out_buf = vec![0.0f32; frame];
	for start in (0..padded_len).step_by(frame) {
		for (i, slot) in in_buf.iter_mut().enumerate() {
			*slot = input.get(start + i).map_or(0.0, |s| s * I16_SCALE);
		}
		state.process_frame(&mut out_buf, &in_buf);
		out.extend(out_buf.iter().map(|s| (s / I16_SCALE).clamp(-1.0, 1.0)));
	}
	out.drain(..frame);
	out.truncate(input.len());
	out
}

/// [`denoise_16k`] over a 16 kHz mono WAV (what every transcription
/// path receives), re-encoded as the same format.
pub fn denoise_wav(wav: &[u8]) -> Result<Vec<u8>, String> {
	let samples = crate::stt::wav_to_samples(wav)?;
	crate::voice::encode_wav_16k(&denoise_16k(&samples))
}

#[cfg(test)]
mod tests {
	use super::{denoise_16k, denoise_48k};

	fn rms(samples: &[f32]) -> f32 {
		(samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
	}

	/// Deterministic white noise (xorshift), amplitude ~0.1.
	fn noise(len: usize) -> Vec<f32> {
		let mut x: u32 = 0x9e37_79b9;
		(0..len)
			.map(|_| {
				x ^= x << 13;
				x ^= x >> 17;
				x ^= x << 5;
				(x as f32 / u32::MAX as f32 - 0.5) * 0.2
			})
			.collect()
	}

	#[test]
	fn keeps_the_length_of_any_input() {
		for len in [0, 1, 479, 480, 481, 16_000, 16_123] {
			assert_eq!(denoise_16k(&vec![0.01; len]).len(), len, "len {len}");
		}
		assert_eq!(denoise_48k(&vec![0.01; 1_000]).len(), 1_000);
	}

	#[test]
	fn steady_noise_is_suppressed() {
		let input = noise(3 * 16_000);
		let out = denoise_16k(&input);
		// judge the second half, after RNNoise's estimate has settled
		let half = input.len() / 2;
		assert!(
			rms(&out[half..]) < rms(&input[half..]) * 0.5,
			"noise not suppressed: {} -> {}",
			rms(&input[half..]),
			rms(&out[half..])
		);
		assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
	}

	#[test]
	fn silence_stays_silent() {
		let out = denoise_16k(&vec![0.0; 16_000]);
		assert!(rms(&out) < 1e-4, "{}", rms(&out));
	}

	#[test]
	fn denoised_wav_round_trips() {
		let wav = crate::voice::encode_wav_16k(&noise(16_000)).expect("encode");
		let cleaned = super::denoise_wav(&wav).expect("denoise");
		assert_eq!(cleaned.len(), wav.len());
	}
}
