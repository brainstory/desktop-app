//! On-device transcription through Apple's Speech framework (SpeechAnalyzer,
//! macOS 26+). Sits next to the whisper engine; chosen per the `ai_stt_engine`
//! setting with automatic fallback to whisper in `auto` mode.

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod macos {
	use speech::prelude::*;

	pub struct AppleSpeechStatus {
		pub available: bool,
		/// Speech-recognition TCC permission granted to this app.
		pub authorized: bool,
		/// Locales SpeechTranscriber can handle (30+ on macOS 26).
		pub supported_locales: Vec<String>,
		/// Subset of supported locales whose assets are installed.
		pub installed_locales: Vec<String>,
	}

	pub fn status() -> AppleSpeechStatus {
		if !crate::apple::os_at_least_26() {
			return AppleSpeechStatus {
				available: false,
				authorized: false,
				supported_locales: Vec::new(),
				installed_locales: Vec::new(),
			};
		}
		let available = SpeechTranscriber::is_available();
		let authorized = SpeechRecognizer::authorization_status().is_authorized();
		let (supported_locales, installed_locales) = if available {
			match (
				SpeechTranscriber::supported_locales(),
				SpeechTranscriber::installed_locales(),
			) {
				(Ok(s), Ok(i)) => (s, i),
				(Err(e), _) | (_, Err(e)) => {
					log::warn!("failed to enumerate speech locales: {e}");
					(Vec::new(), Vec::new())
				}
			}
		} else {
			(Vec::new(), Vec::new())
		};
		AppleSpeechStatus {
			available,
			authorized,
			supported_locales,
			installed_locales,
		}
	}

	fn map_error(e: SpeechError) -> String {
		match e {
			SpeechError::NotAuthorized(_) => "speech recognition permission was denied - allow it in System Settings > Privacy & Security > Speech Recognition".into(),
			SpeechError::RecognizerUnavailable(_) => "Apple Speech transcription is unavailable on this system (requires macOS 26)".into(),
			other => format!("Apple Speech transcription failed: {other}"),
		}
	}

	/// Transcribe a 16 kHz mono WAV buffer on-device. Blocking (authorization
	/// can wait on the user, analysis runs synchronously); call from
	/// `spawn_blocking`.
	pub fn transcribe(wav: &[u8], locale: &str) -> Result<String, String> {
		if !crate::apple::os_at_least_26() {
			return Err("Apple Speech transcription requires macOS 26 or newer".into());
		}

		let auth = SpeechRecognizer::authorization_status();
		if !auth.is_authorized() {
			// NotDetermined triggers the system prompt once; Denied returns
			// promptly and the error tells the user where to flip it back.
			let after = SpeechRecognizer::request_authorization().map_err(map_error)?;
			if !after.is_authorized() {
				return Err("speech recognition permission was denied - allow it in System Settings > Privacy & Security > Speech Recognition".into());
			}
		}

		let locale = if locale.is_empty() { "en-US" } else { locale };
		let resolved = SpeechTranscriber::supported_locale_equivalent_to(locale)
			.map_err(map_error)?
			.unwrap_or_else(|| locale.to_string());

		// The analyzer takes a file URL; park the WAV in the temp dir.
		let path =
			std::env::temp_dir().join(format!("brainstory-stt-{}.wav", uuid::Uuid::new_v4()));
		std::fs::write(&path, wav).map_err(|e| format!("failed to write temp audio: {e}"))?;
		let result = analyze_file(&path, &resolved);
		let _ = std::fs::remove_file(&path);
		result
	}

	fn analyze_file(path: &std::path::Path, locale: &str) -> Result<String, String> {
		let transcriber = SpeechTranscriber::new(locale, SpeechTranscriberPreset::Transcription);
		let analyzer = SpeechAnalyzer::new([SpeechModuleDescriptor::from(&transcriber)]);
		let output = analyzer.analyze_in_path(path).map_err(map_error)?;

		// The Transcription preset emits sentence-level final results; join
		// them into one transcript. If nothing looked final, fall back to all
		// results rather than dropping partial output.
		let mut finals = Vec::new();
		let mut any = Vec::new();
		for module in &output.modules {
			if let SpeechAnalyzerModuleResults::SpeechTranscriber(results) = &module.results {
				for result in results {
					any.push(result.transcript().to_string());
					if result.is_final {
						finals.push(result.transcript().to_string());
					}
				}
			}
		}
		let parts = if finals.is_empty() { any } else { finals };
		Ok(join_transcript(parts))
	}

	/// Sentence results already carry leading/trailing spacing; normalize
	/// instead of blindly joining (which would double spaces).
	fn join_transcript(parts: Vec<String>) -> String {
		let mut out = String::new();
		for part in parts {
			let trimmed = part.trim();
			if trimmed.is_empty() {
				continue;
			}
			if !out.is_empty() {
				out.push(' ');
			}
			out.push_str(trimmed);
		}
		out
	}

	#[cfg(test)]
	mod tests {
		use super::join_transcript;

		#[test]
		fn joins_sentence_parts_without_double_spaces() {
			let parts = vec!["Hello there.".to_string(), " This is a test.".to_string()];
			assert_eq!(join_transcript(parts), "Hello there. This is a test.");
		}

		#[test]
		fn skips_empty_parts() {
			let parts = vec!["".to_string(), "  ".to_string(), "Words".to_string()];
			assert_eq!(join_transcript(parts), "Words");
		}

		#[test]
		fn empty_input_is_empty() {
			assert_eq!(join_transcript(Vec::new()), "");
		}
	}
}

#[cfg(not(target_os = "macos"))]
pub use other::*;

#[cfg(not(target_os = "macos"))]
mod other {
	pub struct AppleSpeechStatus {
		pub available: bool,
		pub authorized: bool,
		pub supported_locales: Vec<String>,
		pub installed_locales: Vec<String>,
	}

	pub fn status() -> AppleSpeechStatus {
		AppleSpeechStatus {
			available: false,
			authorized: false,
			supported_locales: Vec::new(),
			installed_locales: Vec::new(),
		}
	}

	pub fn transcribe(_wav: &[u8], _locale: &str) -> Result<String, String> {
		Err("Apple Speech transcription is only available on macOS 26+".into())
	}
}
