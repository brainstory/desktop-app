//! Env-gated end-to-end test for the Apple Speech engine (SpeechAnalyzer,
//! macOS 26+). Run on a Mac with the macOS 26 SDK, speech-recognition
//! permission granted to the test runner, and en-US dictation assets:
//!
//!     APPLE_STT_SMOKE=1 cargo test --test apple_stt_smoke -- --nocapture
//!
//! Skips silently otherwise (no env var, wrong OS, or no permission).

use brainstory_lib::{apple, stt_apple};
use std::process::Command;

fn render_wav(text: &str) -> Result<Vec<u8>, String> {
	let aiff = std::env::temp_dir().join("brainstory-apple-smoke.aiff");
	let wav = std::env::temp_dir().join("brainstory-apple-smoke.wav");
	let _ = std::fs::remove_file(&aiff);
	let _ = std::fs::remove_file(&wav);
	Command::new("/usr/bin/say")
		.args(["-o", aiff.to_str().ok_or("path")?, text])
		.status()
		.map_err(|e| e.to_string())?
		.success()
		.then_some(())
		.ok_or_else(|| "say failed".to_string())?;
	// 16 kHz mono 16-bit WAV, the format the recorder produces
	Command::new("/usr/bin/afconvert")
		.args([
			"-f",
			"WAVE",
			"-d",
			"LEI16@16000",
			"-c",
			"1",
			aiff.to_str().ok_or("path")?,
			wav.to_str().ok_or("path")?,
		])
		.status()
		.map_err(|e| e.to_string())?
		.success()
		.then_some(())
		.ok_or_else(|| "afconvert failed".to_string())?;
	std::fs::read(&wav).map_err(|e| e.to_string())
}

#[test]
fn transcribes_a_synthesized_utterance() {
	if std::env::var("APPLE_STT_SMOKE").is_err() {
		return;
	}
	if !apple::os_at_least_26() {
		eprintln!("skip: requires macOS 26+");
		return;
	}
	let wav = render_wav(
		"This on-device transcription test mentions brainstory several times, brainstory, brainstory.",
	)
	.expect("render test audio");
	let transcript = stt_apple::transcribe(&wav, "en-US").expect("apple transcription failed");
	println!("apple transcript: {transcript:?}");
	// `say` pronounces "brainstory" as two words, so accept either spelling
	let lower = transcript.to_lowercase();
	assert!(
		lower.contains("brainstory") || lower.contains("brain story"),
		"transcript did not contain the target word"
	);
}
