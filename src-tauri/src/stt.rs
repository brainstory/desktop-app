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
					if is_speech(&text, segment.no_speech_probability()) {
						transcript.push_str(&text);
					}
				}
			}
		}
		Ok(transcript.trim().to_string())
	}
}

/// Whisper rates each segment's chance of being no speech at all. On
/// silence and room noise (base.en) it says 0.86-0.95 while still
/// emitting text ("you", "Sizzling."); real speech, even very quiet,
/// rates below 0.1.
const NO_SPEECH_DROP: f32 = 0.6;

/// Whether a whisper segment is the user's words rather than what
/// whisper makes of silence or noise.
fn is_speech(segment: &str, no_speech_probability: f32) -> bool {
	no_speech_probability <= NO_SPEECH_DROP && !is_non_speech_annotation(segment)
}

/// Whisper writes what it hears in silence or noise as a bracketed
/// annotation segment ("[BLANK_AUDIO]", "[MUSIC]", "(wind blowing)").
/// Those are not the user's words and must not reach the conversation.
fn is_non_speech_annotation(segment: &str) -> bool {
	let text = segment.trim();
	let bracketed = |open: char, close: char| {
		text.len() > 2
			&& text.starts_with(open)
			&& text.ends_with(close)
			&& !text[1..text.len() - 1].contains([open, close])
	};
	bracketed('[', ']') || bracketed('(', ')')
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

/// Hard cap on audio duration at every decode/convert boundary: decoded
/// audio may not exceed this many seconds at its source rate, and
/// resampled 16 kHz output may not exceed `MAX_AUDIO_SECS * 16_000`
/// samples. Mirrors the device-side capture cap (`voice.rs`
/// `MAX_CAPTURE_SECS`); a test pins the two together.
pub(crate) const MAX_AUDIO_SECS: usize = 300;

/// Widest sample rate accepted from WAV metadata or capture devices:
/// 1 kHz covers telephone audio, 192 kHz covers professional audio.
/// Rates outside the range are malformed metadata, and a floor on the
/// rate bounds how far resampling to 16 kHz can multiply the input.
const MIN_SAMPLE_RATE: u32 = 1_000;
const MAX_SAMPLE_RATE: u32 = 192_000;

/// The resampling target whisper requires.
const TARGET_RATE: u32 = 16_000;

/// Pure, allocation-free validation of a resampling budget: may
/// `input_samples` mono frames recorded at `sample_rate` be converted to
/// 16 kHz? Returns the exact output length (`input * 16_000 / rate`,
/// floored) for the caller to allocate with. The rate range, the
/// `MAX_AUDIO_SECS` duration bound at the source rate and the
/// `MAX_AUDIO_SECS * 16_000` bound on the output are all enforced with
/// checked integer arithmetic, so pathological metadata can neither
/// overflow the budget computation nor drive a huge allocation.
pub(crate) fn checked_resample_len(
	input_samples: usize,
	sample_rate: u32,
) -> Result<usize, String> {
	if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
		return Err(format!(
			"unsupported sample rate {sample_rate} Hz (supported {MIN_SAMPLE_RATE}-{MAX_SAMPLE_RATE} Hz)"
		));
	}
	let too_long = || format!("audio too long: more than {MAX_AUDIO_SECS} s at {sample_rate} Hz");
	let max_input = sample_rate
		.checked_mul(MAX_AUDIO_SECS as u32)
		.ok_or_else(too_long)? as usize;
	if input_samples > max_input {
		return Err(format!(
			"audio too long: {input_samples} samples at {sample_rate} Hz exceeds the {MAX_AUDIO_SECS} s limit ({max_input} samples)"
		));
	}
	let out_len = input_samples
		.checked_mul(TARGET_RATE as usize)
		.and_then(|samples| samples.checked_div(sample_rate as usize))
		.ok_or_else(too_long)?;
	let max_output = MAX_AUDIO_SECS * TARGET_RATE as usize;
	if out_len > max_output {
		return Err(format!(
			"audio too long: resampled output of {out_len} samples exceeds the {MAX_AUDIO_SECS} s limit ({max_output} samples)"
		));
	}
	Ok(out_len)
}

/// Decode a WAV file into 16 kHz mono f32 samples suitable for whisper.
/// Handles mono folding and naive linear resampling. Malformed or
/// unsupported files produce errors, never silent empty transcripts.
/// Audio longer than `MAX_AUDIO_SECS` is rejected before the resample
/// allocation.
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
	if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
		return Err(format!(
			"invalid WAV: sample rate {sample_rate} Hz outside the supported {MIN_SAMPLE_RATE}-{MAX_SAMPLE_RATE} Hz range"
		));
	}
	// Header-level duration bound: the frame count is metadata like the
	// rate, so reject over-long audio before collecting samples. The
	// authoritative check on the actual collected count runs in
	// resample_to_16k before its allocation.
	checked_resample_len(reader.duration() as usize, sample_rate)?;

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

	resample_to_16k(fold_to_mono(samples, channels), sample_rate)
}

/// Fold interleaved multi-channel samples to mono by averaging each
/// frame. Each frame is divided by its own length: a truncated file or
/// capture can end mid-frame, and dividing a short frame by the full
/// channel count would produce a spurious volume dip. Shared by WAV
/// decoding and microphone capture.
pub(crate) fn fold_to_mono(samples: Vec<f32>, channels: usize) -> Vec<f32> {
	if channels <= 1 {
		return samples;
	}
	samples
		.chunks(channels)
		.map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
		.collect()
}

/// Naive linear resampling to 16 kHz. The output length is validated
/// and computed by [`checked_resample_len`] before any allocation, so
/// metadata-controlled rates or durations cannot blow up the buffer.
pub(crate) fn resample_to_16k(mono: Vec<f32>, sample_rate: u32) -> Result<Vec<f32>, String> {
	let out_len = checked_resample_len(mono.len(), sample_rate)?;
	if sample_rate == TARGET_RATE || mono.is_empty() {
		return Ok(mono);
	}
	let ratio = f64::from(sample_rate) / f64::from(TARGET_RATE);
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
	if !crate::models::has_http_scheme(base) {
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
	let body = crate::llm::read_body_capped(response, 4 * 1024 * 1024, 60, 120).await;
	let value: serde_json::Value =
		serde_json::from_str(&body).map_err(|e| format!("invalid STT response: {e}"))?;
	value["text"]
		.as_str()
		.map(|s| s.to_string())
		.ok_or_else(|| "external STT returned no text".into())
}

#[cfg(test)]
mod tests {
	use super::{
		checked_resample_len, is_non_speech_annotation, is_speech, resample_to_16k, wav_to_samples,
		whisper_language,
	};

	#[test]
	fn segments_whisper_rates_as_no_speech_are_dropped() {
		// what base.en returned for digital silence, noise, and speech
		assert!(!is_speech(" you", 0.94));
		assert!(!is_speech(" Sizzling.", 0.86));
		assert!(!is_speech(" [BLANK_AUDIO]", 0.0));
		assert!(is_speech(" I'm testing the BrainStory app.", 0.01));
		assert!(is_speech(" Yes.", 0.09));
	}

	#[test]
	fn whisper_non_speech_annotations_are_dropped() {
		for segment in [
			" [BLANK_AUDIO]",
			"[MUSIC]",
			" (wind blowing)",
			"[ Silence ]\n",
		] {
			assert!(is_non_speech_annotation(segment), "{segment:?}");
		}
		for segment in [
			" I'm testing the app",
			" [laughs] that was fun",
			" it costs (about) ten",
			" (a) and (b)",
			"[]",
			"",
		] {
			assert!(!is_non_speech_annotation(segment), "{segment:?}");
		}
	}

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

	#[test]
	fn rejects_pathological_wav_sample_rates_before_allocating() {
		// A WAV announcing 1 Hz turns every input sample into 16_000
		// output samples: 64 samples resample to a million. The rate
		// must be rejected before any resample allocation.
		let bytes = wav_bytes(mono_spec(1, 1), &[0i16; 64]);
		match wav_to_samples(&bytes) {
			Err(e) => assert!(e.contains("sample rate"), "{e}"),
			Ok(samples) => panic!(
				"rate-1 WAV accepted: {} output samples from 64 input (output length = input * 16000, unbounded)",
				samples.len()
			),
		}
		// below-minimum but nonzero rate: same rejection
		let bytes = wav_bytes(mono_spec(500, 1), &[0i16; 1_000]);
		match wav_to_samples(&bytes) {
			Err(e) => assert!(e.contains("sample rate"), "{e}"),
			Ok(samples) => panic!(
				"rate-500 WAV accepted: {} output samples from 1_000 input",
				samples.len()
			),
		}
	}

	#[test]
	fn rejects_zero_sample_rate_wav() {
		// patch the fmt chunk consistently (rate AND byte rate, which
		// hound cross-checks) so the zero reaches our validation
		let mut bytes = wav_bytes(mono_spec(16_000, 1), &[0, 0]);
		bytes[24..28].copy_from_slice(&0u32.to_le_bytes());
		bytes[28..32].copy_from_slice(&0u32.to_le_bytes());
		match wav_to_samples(&bytes) {
			Err(e) => assert!(e.contains("zero sample rate"), "{e}"),
			Ok(samples) => panic!("zero-rate WAV accepted: {} samples", samples.len()),
		}
	}

	#[test]
	fn resample_bound_is_exactly_300_seconds() {
		// exactly at the cap passes...
		let at_bound = resample_to_16k(vec![0.0; 300_000], 1_000).expect("300 s at 1 kHz");
		assert_eq!(at_bound.len(), 4_800_000);
		// ...one sample more is rejected before allocating
		match resample_to_16k(vec![0.0; 300_001], 1_000) {
			Err(e) => assert!(e.contains("too long"), "{e}"),
			Ok(out) => panic!(
				"over-bound resample accepted: {} output samples (300_001 * 16000 / 1000, unbounded)",
				out.len()
			),
		}
	}

	#[test]
	fn decodes_common_rates_8k_16k_44k1_48k() {
		for rate in [8_000u32, 16_000, 44_100, 48_000] {
			let bytes = wav_bytes(mono_spec(rate, 1), &vec![0i16; rate as usize]);
			let samples = wav_to_samples(&bytes).unwrap_or_else(|e| panic!("{rate} Hz: {e}"));
			assert!(
				(15_800..=16_200).contains(&samples.len()),
				"{rate} Hz decoded to {} samples",
				samples.len()
			);
		}
	}

	#[test]
	fn folds_stereo_to_mono_at_48k_before_resampling() {
		let mut frames = Vec::new();
		for _ in 0..6 {
			frames.extend_from_slice(&[16384, -16384]); // L/R cancel
		}
		let bytes = wav_bytes(mono_spec(48_000, 2), &frames);
		let samples = wav_to_samples(&bytes).expect("decode");
		assert_eq!(
			samples.len(),
			2,
			"6 frames at 48 kHz -> 2 samples at 16 kHz"
		);
		assert!(samples.iter().all(|s| s.abs() < 1e-3), "{samples:?}");
	}

	#[test]
	fn checked_resample_len_accepts_the_documented_rate_range() {
		// one second at any supported rate resamples to 16_000 samples
		for rate in [1_000u32, 8_000, 16_000, 44_100, 48_000, 192_000] {
			assert_eq!(
				checked_resample_len(rate as usize, rate).unwrap(),
				16_000,
				"{rate} Hz"
			);
		}
	}

	#[test]
	fn checked_resample_len_rejects_zero_and_out_of_range_rates() {
		for rate in [0, 1, 500, 999, 193_000, 1_000_000, u32::MAX] {
			assert!(
				checked_resample_len(1_000, rate).is_err(),
				"{rate} Hz accepted"
			);
		}
	}

	#[test]
	fn checked_resample_len_allows_exactly_300s_and_rejects_one_sample_more() {
		for (rate, at_bound) in [
			(1_000u32, 300_000usize),
			(16_000, 4_800_000),
			(44_100, 13_230_000),
			(48_000, 14_400_000),
			(192_000, 57_600_000),
		] {
			assert_eq!(
				checked_resample_len(at_bound, rate).unwrap(),
				4_800_000,
				"{rate} Hz at bound"
			);
			assert!(
				checked_resample_len(at_bound + 1, rate).is_err(),
				"{rate} Hz one over accepted"
			);
		}
	}

	#[test]
	fn checked_resample_len_floors_the_output_length() {
		assert_eq!(checked_resample_len(1, 48_000).unwrap(), 0);
		assert_eq!(checked_resample_len(3, 48_000).unwrap(), 1);
		assert_eq!(checked_resample_len(22_050, 44_100).unwrap(), 8_000);
		// upsampling in frequency terms (8k -> 16k) doubles
		assert_eq!(checked_resample_len(4_000, 8_000).unwrap(), 8_000);
		// the resampler allocates exactly the computed length
		assert_eq!(
			resample_to_16k(vec![0.0; 22_050], 44_100).unwrap().len(),
			checked_resample_len(22_050, 44_100).unwrap()
		);
	}

	#[test]
	fn checked_resample_len_survives_huge_sample_counts_without_overflow() {
		for (samples, rate) in [
			(usize::MAX, 1_000u32),
			(usize::MAX, 16_000),
			(usize::MAX, 192_000),
			(usize::MAX / 2, 44_100),
			(u32::MAX as usize, 48_000),
		] {
			assert!(
				checked_resample_len(samples, rate).is_err(),
				"{samples} samples at {rate} Hz not rejected"
			);
		}
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
