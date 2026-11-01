//! End-to-end check of the local speech pipeline on real speech:
//! band-limited resampling, Silero VAD, RNNoise and Parakeet (ONNX
//! Runtime) - plus whisper.cpp in the same process when a whisper model
//! is given, proving ONNX Runtime and ggml coexist.
//!
//! Needs model files and a recording, so it only runs when pointed at
//! them (skipped otherwise):
//!
//! - `PARAKEET_MODEL_DIR`: a folder with istupakov/parakeet-tdt-0.6b-v3-onnx's
//!   `encoder-model.int8.onnx`, `decoder_joint-model.int8.onnx`,
//!   `nemo128.onnx` and `vocab.txt`
//! - `SPEECH_WAV`: a WAV of someone saying "The quick brown fox jumps
//!   over the lazy dog." (any rate/channels), e.g.
//!   `espeak-ng -w speech.wav "The quick brown fox jumps over the lazy dog."`
//! - `WHISPER_MODEL_PATH` (optional): a whisper ggml model
#![allow(linker_messages)]

use brainstory_lib::stt::{wav_to_samples, SttEngine, TRANSCRIPTION_CANCELLED};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const RATE: usize = 16_000;

fn env_path(var: &str) -> Option<std::path::PathBuf> {
	std::env::var_os(var).map(std::path::PathBuf::from)
}

/// `secs` of silence, the speech, a long pause, the speech again, and
/// a trailing pause: something for VAD to cut and for Parakeet to read
/// across a chunk boundary.
fn recording(speech: &[f32]) -> Vec<f32> {
	let mut out = vec![0.0; RATE];
	out.extend_from_slice(speech);
	out.extend(std::iter::repeat_n(0.0, 3 * RATE));
	out.extend_from_slice(speech);
	out.extend(std::iter::repeat_n(0.0, RATE));
	out
}

fn normalized(text: &str) -> String {
	text.to_lowercase()
		.chars()
		.filter(|c| c.is_alphanumeric() || c.is_whitespace())
		.collect()
}

fn assert_heard_twice(engine: &str, transcript: &str) {
	let text = normalized(transcript);
	println!("{engine}: {transcript:?}");
	assert_eq!(
		text.matches("brown fox").count(),
		2,
		"{engine} should hear both sentences: {transcript:?}"
	);
	assert!(text.contains("lazy dog"), "{engine}: {transcript:?}");
}

#[test]
fn speech_pipeline_transcribes_real_speech() {
	let (Some(model_dir), Some(wav)) = (env_path("PARAKEET_MODEL_DIR"), env_path("SPEECH_WAV"))
	else {
		eprintln!("skipped: set PARAKEET_MODEL_DIR and SPEECH_WAV");
		return;
	};
	let speech = wav_to_samples(&std::fs::read(wav).expect("read SPEECH_WAV")).expect("decode");
	let samples = recording(&speech);

	// VAD finds the two sentences and drops the pauses around them
	let vad_dir = tempfile::tempdir().expect("tempdir");
	let ranges = brainstory_lib::vad::speech_ranges(vad_dir.path(), &samples).expect("VAD runs");
	let speech_len: usize = ranges.iter().map(|r| r.len()).sum();
	println!("VAD: {ranges:?}");
	assert!(!ranges.is_empty(), "no speech found");
	assert!(
		speech_len < samples.len() - 3 * RATE,
		"the pauses should be cut: {speech_len} of {} samples kept",
		samples.len()
	);
	assert!(
		ranges.first().unwrap().start >= RATE / 2,
		"leading silence kept: {ranges:?}"
	);

	let cancel = Arc::new(AtomicBool::new(false));
	let parakeet = SttEngine::load_parakeet(&model_dir, "parakeet-tdt-0.6b-v3").expect("load");
	let transcript = parakeet
		.transcribe_speech(&samples, Some(&ranges), "en-US", &cancel)
		.expect("transcribe");
	assert_heard_twice("parakeet", &transcript);

	// without VAD the whole recording is chunked and still transcribed
	let transcript = parakeet
		.transcribe_speech(&samples, None, "en-US", &cancel)
		.expect("transcribe without VAD");
	assert_heard_twice("parakeet (no VAD)", &transcript);

	// RNNoise-cleaned audio stays intelligible
	let cleaned = brainstory_lib::denoise::denoise_16k(&samples);
	let transcript = parakeet
		.transcribe_speech(&cleaned, Some(&ranges), "en-US", &cancel)
		.expect("transcribe denoised");
	assert_heard_twice("parakeet (denoised)", &transcript);

	// a cancelled run reports the cancellation, not a transcript
	let cancelled = Arc::new(AtomicBool::new(true));
	assert_eq!(
		parakeet.transcribe_speech(&samples, Some(&ranges), "en-US", &cancelled),
		Err(TRANSCRIPTION_CANCELLED.to_string())
	);

	// whisper.cpp (ggml) in the same process as ONNX Runtime
	if let Some(whisper_path) = env_path("WHISPER_MODEL_PATH") {
		let whisper = SttEngine::load(&whisper_path, "whisper-test-en").expect("load whisper");
		let transcript = whisper
			.transcribe_speech(&samples, Some(&ranges), "en-US", &cancel)
			.expect("whisper transcribe");
		assert_heard_twice("whisper", &transcript);
		let transcript = whisper
			.transcribe_speech(&samples, None, "en-US", &cancel)
			.expect("whisper transcribe without VAD");
		assert_heard_twice("whisper (no VAD)", &transcript);
	}
}
