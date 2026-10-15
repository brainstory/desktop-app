//! Sampling profiles shared by the engines, with a runtime override
//! file (`<app data>/prompts/sampling.json`) for experimenting without
//! a binary release.

use super::Sampling;

/// The override in effect, loaded once at startup: the profile defaults
/// patched by the JSON file, or the defaults when the file is absent,
/// unreadable, oversized or out of range.
static OVERRIDES: std::sync::OnceLock<[Sampling; 2]> = std::sync::OnceLock::new();

/// Largest override file accepted (the shipped defaults serialize to a
/// few hundred bytes).
const MAX_OVERRIDE_BYTES: u64 = 64 * 1024;

/// Record the overrides from `<app data>/prompts/sampling.json` for the
/// process (later calls are ignored). Invalid or out-of-range values
/// fall back to the defaults field by field.
pub fn init_overrides(app_data_dir: &std::path::Path) {
	let path = app_data_dir.join("prompts").join("sampling.json");
	let loaded = match std::fs::metadata(&path) {
		Ok(meta) if meta.len() <= MAX_OVERRIDE_BYTES => match std::fs::read_to_string(&path) {
			Ok(text) => serde_json::from_str::<serde_json::Value>(&text).ok(),
			Err(e) => {
				log::warn!("sampling override unreadable ({e}); using the built-in profiles");
				None
			}
		},
		Ok(_) => {
			log::warn!("sampling override is oversized; using the built-in profiles");
			None
		}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
		Err(e) => {
			log::warn!("sampling override unreadable ({e}); using the built-in profiles");
			None
		}
	};
	let patch = |mut profile: Sampling, body: &serde_json::Value| {
		let f32_field = |name: &str, min: f32, max: f32| -> Option<f32> {
			body[name]
				.as_f64()
				.map(|v| v as f32)
				.filter(|v| (min..=max).contains(v))
		};
		if let Some(v) = f32_field("temperature", 0.0, 2.0) {
			profile.temperature = v;
		}
		if let Some(v) = body["top_k"]
			.as_i64()
			.map(|v| v as i32)
			.filter(|v| (1..=256).contains(v))
		{
			profile.top_k = v;
		}
		if let Some(v) = f32_field("top_p", 0.05, 1.0) {
			profile.top_p = v;
		}
		if let Some(v) = f32_field("min_p", 0.0, 0.5) {
			profile.min_p = v;
		}
		if let Some(v) = f32_field("repeat_penalty", 0.5, 2.0) {
			profile.repeat_penalty = v;
		}
		if let Some(v) = body["repeat_last_n"]
			.as_i64()
			.map(|v| v as i32)
			.filter(|v| (0..=2048).contains(v))
		{
			profile.repeat_last_n = v;
		}
		profile
	};
	let body = loaded.clone().unwrap_or(serde_json::json!({}));
	let profiles = [
		patch(Sampling::for_request(false), &body["reply"]),
		patch(Sampling::for_request(true), &body["summary"]),
	];
	if loaded.is_some() {
		log::info!("sampling overrides applied from {}", path.display());
	}
	let _ = OVERRIDES.set(profiles);
}

/// The sampling profile for one generation (index 0 = reply, 1 = summary).
pub fn effective(summarize: bool) -> Sampling {
	OVERRIDES
		.get()
		.map(|profiles| profiles[usize::from(summarize)])
		.unwrap_or_else(|| Sampling::for_request(summarize))
}

#[cfg(test)]
mod tests {
	use super::{effective, init_overrides, Sampling};

	#[test]
	fn defaults_differ_between_replies_and_summaries() {
		let reply = Sampling::for_request(false);
		let summary = Sampling::for_request(true);
		assert!(reply.temperature > summary.temperature);
		assert!(reply.top_k > summary.top_k);
		assert!(reply.repeat_penalty > summary.repeat_penalty);
		assert!(summary.repeat_last_n > reply.repeat_last_n);
	}

	#[test]
	fn override_files_patch_within_range_and_ignore_garbage() {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::create_dir_all(dir.path().join("prompts")).expect("prompts dir");
		std::fs::write(
			dir.path().join("prompts/sampling.json"),
			r#"{"reply": {"temperature": 0.9}, "summary": {"top_k": 900, "temperature": -1}}"#,
		)
		.expect("write override");
		// OnceLock: this test must run before any other that initializes
		// overrides; the test binary is single-threaded per test but the
		// static is shared, so only ONE test may initialize it. That is
		// this one.
		init_overrides(dir.path());
		let reply = effective(false);
		assert!((reply.temperature - 0.9).abs() < 1e-6, "patched: {reply:?}");
		let summary = effective(true);
		assert_eq!(
			summary.top_k,
			Sampling::for_request(true).top_k,
			"out of range ignored"
		);
		assert_eq!(
			summary.temperature,
			Sampling::for_request(true).temperature,
			"negative ignored"
		);
	}
}
