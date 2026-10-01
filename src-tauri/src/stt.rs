use std::path::Path;
use std::sync::Arc;

pub struct SttEngine {
	ctx: Arc<whisper_rs::WhisperContext>,
	pub model_id: String,
}

impl SttEngine {
	pub fn load(path: &Path, model_id: &str) -> Result<Self, String> {
		let params = whisper_rs::WhisperContextParameters::default();
		let ctx = whisper_rs::WhisperContext::new_with_params(path, params)
			.map_err(|e| format!("failed to load whisper model {path:?}: {e}"))?;
		Ok(Self {
			ctx: Arc::new(ctx),
			model_id: model_id.to_string(),
		})
	}

	/// Transcribe 16 kHz mono PCM samples. Blocking; run off the main
	/// thread. `language` is a BCP-47 locale ("de-DE"); its primary
	/// subtag selects the whisper language. English-only models (the
	/// `*.en` builds) always transcribe as English.
	pub fn transcribe(&self, samples: &[f32], language: &str) -> Result<String, String> {
		let mut state = self.ctx.create_state().map_err(|e| e.to_string())?;
		let mut params =
			whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 5 });
		let language = whisper_language(language, self.model_id.ends_with("-en"));
		params.set_language(Some(&language));
		params.set_n_threads(whisper_threads());
		params.set_translate(false);
		params.set_print_progress(false);
		params.set_print_special(false);
		params.set_print_realtime(false);
		params.set_print_timestamps(false);

		state
			.full(params, samples)
			.map_err(|e| format!("transcription failed: {e}"))?;

		let n_segments = state.full_n_segments();
		let mut transcript = String::new();
		for i in 0..n_segments {
			if let Some(segment) = state.get_segment(i) {
				if let Ok(text) = segment.to_str_lossy() {
					transcript.push_str(&text);
				}
			}
		}
		Ok(transcript.trim().to_string())
	}
}

/// Whisper's CPU thread count: one per physical core. Hyperthreads don't
/// help its memory-bound compute, but halving the logical count (the old
/// heuristic) also halved it on CPUs without SMT, such as Apple Silicon.
fn whisper_threads() -> i32 {
	num_cpus::get_physical().clamp(1, i32::MAX as usize) as i32
}

/// The whisper language for a BCP-47 locale ("de-DE" -> "de",
/// "zh_Hans" -> "zh"). English-only models (the `*.en` builds) always get
/// "en". Primary subtags whisper spells differently are mapped
/// ("nb" -> "no", ...); anything whisper does not know becomes "auto"
/// (detect from the audio) instead of reaching whisper.cpp, which fails
/// the whole transcription on an unknown language.
fn whisper_language(locale: &str, english_only: bool) -> String {
	if english_only {
		return "en".into();
	}
	let primary = locale
		.split(['-', '_'])
		.find(|s| !s.is_empty())
		.unwrap_or("")
		.to_ascii_lowercase();
	let code = match primary.as_str() {
		"nb" => "no",  // Norwegian Bokmal: whisper only has "no" (and "nn")
		"fil" => "tl", // Filipino: whisper calls it Tagalog
		"iw" => "he",  // deprecated ISO 639 codes some platforms still emit
		"in" => "id",
		"ji" => "yi",
		"jw" => "jv",
		other => other,
	};
	// A primary language subtag is 2-3 letters; the length check also
	// keeps full names ("german"), which whisper_lang_id accepts too, out.
	let known = (2..=3).contains(&code.len())
		&& code.bytes().all(|b| b.is_ascii_lowercase())
		&& whisper_rs::get_lang_id(code).is_some();
	if known {
		code.to_string()
	} else {
		"auto".into()
	}
}

/// Decode a WAV file into 16 kHz mono f32 samples suitable for whisper.
/// Handles mono folding and naive linear resampling. Malformed or
/// unsupported files produce errors, never silent empty transcripts.
pub fn wav_to_samples(bytes: &[u8]) -> Result<Vec<f32>, String> {
	let cursor = std::io::Cursor::new(bytes);
	let mut reader = hound::WavReader::new(cursor).map_err(|e| format!("invalid WAV: {e}"))?;
	let spec = reader.spec();

	let channels = spec.channels as usize;
	let sample_rate = spec.sample_rate;
	if channels == 0 {
		return Err("invalid WAV: zero channels".into());
	}
	if sample_rate == 0 {
		return Err("invalid WAV: zero sample rate".into());
	}

	// Collect with error propagation - silently dropping undecodable
	// samples desyncs stereo channels and hides unsupported formats.
	let samples: Vec<f32> = match spec.sample_format {
		hound::SampleFormat::Float => reader
			.samples::<f32>()
			.collect::<Result<Vec<_>, _>>()
			.map_err(|e| format!("invalid WAV samples: {e}"))?,
		hound::SampleFormat::Int => match spec.bits_per_sample {
			16 => reader
				.samples::<i16>()
				.collect::<Result<Vec<_>, _>>()
				.map_err(|e| format!("invalid WAV samples: {e}"))?
				.into_iter()
				.map(|s| s as f32 / 32768.0)
				.collect(),
			24 | 32 => reader
				.samples::<i32>()
				.collect::<Result<Vec<_>, _>>()
				.map_err(|e| format!("invalid WAV samples: {e}"))?
				.into_iter()
				.map(|s| {
					let scale = if spec.bits_per_sample == 24 {
						8_388_608.0
					} else {
						2_147_483_648.0
					};
					s as f32 / scale
				})
				.collect(),
			other => return Err(format!("unsupported WAV bit depth ({other}-bit integer)")),
		},
	};
	if samples.is_empty() {
		return Err("WAV contains no samples".into());
	}

	// fold channels to mono (divide each frame by its own length: a
	// truncated file can end mid-frame, and dividing a short frame by the
	// full channel count would produce a spurious volume dip)
	let mono: Vec<f32> = if channels > 1 {
		samples
			.chunks(channels)
			.map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
			.collect()
	} else {
		samples
	};

	resample_to_16k(mono, sample_rate)
}

/// Naive linear resampling to 16 kHz.
pub(crate) fn resample_to_16k(mono: Vec<f32>, sample_rate: u32) -> Result<Vec<f32>, String> {
	const TARGET_RATE: u32 = 16_000;
	if sample_rate == TARGET_RATE || mono.is_empty() {
		return Ok(mono);
	}
	let ratio = f64::from(sample_rate) / f64::from(TARGET_RATE);
	if !ratio.is_finite() || ratio <= 0.0 {
		return Err(format!("invalid sample rate {sample_rate}"));
	}
	let out_len = (mono.len() as f64 / ratio) as usize;
	let mut resampled = Vec::with_capacity(out_len);
	let mut src_pos = 0.0f64;
	for _ in 0..out_len {
		let i = src_pos.floor() as usize;
		let frac = src_pos - i as f64;
		let a = mono.get(i).copied().unwrap_or(0.0);
		let b = mono.get(i + 1).copied().unwrap_or(a);
		resampled.push(a + (b - a) * frac as f32);
		src_pos += ratio;
	}
	Ok(resampled)
}

/// OpenAI-compatible audio transcription endpoint
/// (whisper.cpp server, faster-whisper-server, Groq, OpenAI, ...).
pub async fn transcribe_external(
	base_url: &str,
	api_key: &str,
	model: &str,
	wav: Vec<u8>,
) -> Result<String, String> {
	let base = base_url.trim_end_matches('/');
	if !base.starts_with("http://") && !base.starts_with("https://") {
		return Err(format!(
			"invalid STT endpoint URL '{base}' (include http:// or https://)"
		));
	}
	let url = if base.ends_with("/v1") {
		format!("{}/audio/transcriptions", base)
	} else {
		format!("{}/v1/audio/transcriptions", base)
	};

	let model = if model.is_empty() {
		"whisper-1".to_string()
	} else {
		model.to_string()
	};
	let part = reqwest::multipart::Part::bytes(wav)
		.file_name("audio.wav")
		.mime_str("audio/wav")
		.map_err(|e| e.to_string())?;
	let form = reqwest::multipart::Form::new()
		.text("model", model)
		.text("response_format", "json")
		.part("file", part);

	let client = reqwest::Client::builder()
		.connect_timeout(std::time::Duration::from_secs(10))
		// upload + transcription of a few minutes of audio can take a while,
		// but not forever
		.timeout(std::time::Duration::from_secs(180))
		.build()
		.map_err(|e| e.to_string())?;
	let mut request = client
		.post(url)
		.header("User-Agent", crate::models::USER_AGENT)
		.multipart(form);
	if !api_key.is_empty() {
		request = request.bearer_auth(api_key);
	}

	let response = request
		.send()
		.await
		.map_err(|e| format!("request failed: {e}"))?;
	if !response.status().is_success() {
		return Err(format!("external STT error ({})", response.status()));
	}
	// Transcripts are at most a few MB; a cap keeps a broken endpoint from
	// streaming an unbounded "JSON" body into memory.
	let body = crate::llm::read_body_capped(response, 4 * 1024 * 1024, 60).await;
	let value: serde_json::Value =
		serde_json::from_str(&body).map_err(|e| format!("invalid STT response: {e}"))?;
	value["text"]
		.as_str()
		.map(|s| s.to_string())
		.ok_or_else(|| "external STT returned no text".into())
}

#[cfg(test)]
mod tests {
	use super::{resample_to_16k, wav_to_samples, whisper_language};

	#[test]
	fn whisper_language_uses_the_primary_subtag() {
		let f = |locale| whisper_language(locale, false);
		assert_eq!(f("de-DE"), "de");
		assert_eq!(f("en-US"), "en");
		assert_eq!(f("zh_Hans"), "zh");
		assert_eq!(f("fr"), "fr");
		assert_eq!(f("-FR"), "fr");
		assert_eq!(f("PT-br"), "pt");
	}

	#[test]
	fn whisper_language_maps_aliases_and_auto_detects_the_unknown() {
		let f = |locale| whisper_language(locale, false);
		// BCP-47 codes whisper spells differently
		assert_eq!(f("nb-NO"), "no", "Norwegian Bokmal");
		assert_eq!(f("fil-PH"), "tl", "Filipino");
		assert_eq!(f("iw-IL"), "he", "deprecated Hebrew code");
		assert_eq!(f("in-ID"), "id", "deprecated Indonesian code");
		// codes whisper does not know must not reach it (whisper.cpp
		// rejects an unknown language and the transcription fails):
		// fall back to auto-detection instead
		assert_eq!(f("tlh-QO"), "auto", "Klingon is not in whisper's table");
		assert_eq!(f("xx-YY"), "auto");
		assert_eq!(f(""), "auto");
		assert_eq!(f("german"), "auto", "a full name is not a subtag");
		// English-only models stay pinned to English whatever the locale
		assert_eq!(whisper_language("de-DE", true), "en");
		assert_eq!(whisper_language("xx-YY", true), "en");
	}

	fn wav_bytes(spec: hound::WavSpec, samples: &[i16]) -> Vec<u8> {
		let mut cursor = std::io::Cursor::new(Vec::new());
		{
			let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("writer");
			for &s in samples {
				writer.write_sample(s).expect("write");
			}
		}
		cursor.into_inner()
	}

	fn mono_spec(rate: u32, channels: u16) -> hound::WavSpec {
		hound::WavSpec {
			channels,
			sample_rate: rate,
			bits_per_sample: 16,
			sample_format: hound::SampleFormat::Int,
		}
	}

	#[test]
	fn decodes_mono_16k() {
		let bytes = wav_bytes(mono_spec(16_000, 1), &[0, 16384, -16384, 32767]);
		let samples = wav_to_samples(&bytes).expect("decode");
		assert_eq!(samples.len(), 4);
		assert!((samples[1] - 0.5).abs() < 1e-3);
		assert!((samples[2] + 0.5).abs() < 1e-3);
	}

	#[test]
	fn folds_stereo_to_mono() {
		// L=32767, R=-32767 -> average 0
		let bytes = wav_bytes(mono_spec(16_000, 2), &[32767, -32767, 16384, 16384]);
		let samples = wav_to_samples(&bytes).expect("decode");
		assert_eq!(samples.len(), 2);
		assert!(samples[0].abs() < 1e-4);
		assert!((samples[1] - 0.5).abs() < 1e-3);
	}

	#[test]
	fn resamples_44k1_to_16k() {
		let ones = vec![1000i16; 44_100]; // one second
		let bytes = wav_bytes(mono_spec(44_100, 1), &ones);
		let samples = wav_to_samples(&bytes).expect("decode");
		// one second at 16 kHz, small tolerance for the naive resampler
		assert!(
			(15_800..=16_200).contains(&samples.len()),
			"got {} samples",
			samples.len()
		);
	}

	#[test]
	fn rejects_garbage_bytes() {
		assert!(wav_to_samples(b"not a wav file at all").is_err());
		assert!(wav_to_samples(&[]).is_err());
	}

	#[test]
	fn rejects_zero_channels() {
		let bytes = wav_bytes(mono_spec(16_000, 1), &[0, 0]);
		// hand-patch the channel field to 0 in the fmt chunk (byte 22)
		let mut patched = bytes.clone();
		patched[22] = 0;
		assert!(wav_to_samples(&patched).is_err());
	}

	#[test]
	fn rejects_empty_audio() {
		let bytes = wav_bytes(mono_spec(16_000, 1), &[]);
		assert!(wav_to_samples(&bytes).is_err());
	}

	#[test]
	fn resample_passthrough_and_errors() {
		let mono = vec![1.0f32, 2.0, 3.0];
		assert_eq!(resample_to_16k(mono.clone(), 16_000).unwrap(), mono);
		assert!(resample_to_16k(mono.clone(), 0).is_err());
		// upsampling in frequency terms (8k -> 16k) doubles the length
		let up = resample_to_16k(vec![0.0f32, 1.0], 8_000).unwrap();
		assert_eq!(up.len(), 4);
	}
}

#[cfg(test)]
mod wav_depth_tests {
	use super::wav_to_samples;

	type CursorWavWriter<'a> = hound::WavWriter<&'a mut std::io::Cursor<Vec<u8>>>;

	fn wav_bytes(spec: hound::WavSpec, write_samples: &dyn Fn(&mut CursorWavWriter)) -> Vec<u8> {
		let mut cursor = std::io::Cursor::new(Vec::new());
		{
			let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("writer");
			write_samples(&mut writer);
		}
		cursor.into_inner()
	}

	#[test]
	fn decodes_24_and_32bit_and_float_wavs() {
		// 24-bit int: full-scale positive reads as ~1.0
		let spec = hound::WavSpec {
			channels: 1,
			sample_rate: 16_000,
			bits_per_sample: 24,
			sample_format: hound::SampleFormat::Int,
		};
		let bytes = wav_bytes(spec, &|w| {
			w.write_sample(8_388_607i32).unwrap(); // 2^23 - 1
			w.write_sample(-8_388_608i32).unwrap();
		});
		let samples = wav_to_samples(&bytes).expect("decode 24-bit");
		assert!(
			(samples[0] - 1.0).abs() < 1e-4,
			"24-bit full scale: {}",
			samples[0]
		);
		assert!(
			(samples[1] + 1.0).abs() < 1e-4,
			"24-bit negative full scale: {}",
			samples[1]
		);

		// 32-bit int
		let spec = hound::WavSpec {
			channels: 1,
			sample_rate: 16_000,
			bits_per_sample: 32,
			sample_format: hound::SampleFormat::Int,
		};
		let bytes = wav_bytes(spec, &|w| {
			w.write_sample(1_073_741_823i32).unwrap();
		});
		let samples = wav_to_samples(&bytes).expect("decode 32-bit");
		assert!(
			(samples[0] - 0.5).abs() < 1e-6,
			"32-bit half scale: {}",
			samples[0]
		);

		// 32-bit float
		let spec = hound::WavSpec {
			channels: 1,
			sample_rate: 16_000,
			bits_per_sample: 32,
			sample_format: hound::SampleFormat::Float,
		};
		let bytes = wav_bytes(spec, &|w| {
			w.write_sample(0.25f32).unwrap();
			w.write_sample(-0.75f32).unwrap();
		});
		let samples = wav_to_samples(&bytes).expect("decode float");
		assert_eq!(samples, vec![0.25, -0.75]);
	}
}
