use tauri::State;

use crate::db::DEFAULT_LOG_QUESTIONS;
use crate::models::AiSettings;
use crate::types::{LogSettingsItem, NotificationSettingsItem, UserSettings, UserSettingsUser};
use crate::AppState;

#[tauri::command]
pub fn get_user_settings(state: State<'_, AppState>) -> UserSettings {
	let name = state.db.get_setting("user_name").filter(|s| !s.is_empty());
	let timezone = state.db.get_setting("user_timezone").filter(|s| !s.is_empty());
	let reminder_enabled = state
		.db
		.get_setting("reminder_enabled")
		.map(|v| v == "true")
		.unwrap_or(false);
	let reminder_time = state
		.db
		.get_setting("reminder_time")
		.filter(|s| !s.is_empty())
		.unwrap_or_else(|| "09:00".into());

	let enabled_ids = crate::commands::data::enabled_log_ids(&state);
	let log: Vec<LogSettingsItem> = DEFAULT_LOG_QUESTIONS
		.iter()
		.map(|(id, text, label)| LogSettingsItem {
			id: *id,
			label: label.to_string(),
			text: text.to_string(),
			enabled: enabled_ids.contains(id),
		})
		.collect();

	let notifications = vec![NotificationSettingsItem {
		title: "Daily intention reminder".to_string(),
		description: Some(
			"A daily system notification reminding you to set your intention and think out loud."
				.to_string(),
		),
		value: Some(reminder_time),
		value_type: "time".to_string(),
		enabled: reminder_enabled,
	}];

	UserSettings {
		user: UserSettingsUser { name, timezone },
		log,
		notifications,
	}
}

#[tauri::command]
pub fn save_user_settings(
	state: State<'_, AppState>,
	user: Option<serde_json::Value>,
	enabled_log_question_ids: Option<Vec<i64>>,
	notifications: Option<Vec<serde_json::Value>>,
) -> serde_json::Value {
	if let Some(user) = &user {
		if let Some(name) = user["name"].as_str() {
			state.db.set_setting("user_name", name);
		}
		if let Some(timezone) = user["timezone"].as_str() {
			state.db.set_setting("user_timezone", timezone);
		}
	}

	if let Some(ids) = &enabled_log_question_ids {
		let ids: Vec<i64> = {
			let mut ids = ids.clone();
			ids.sort();
			ids.dedup();
			ids
		};
		if ids.is_empty() {
			return serde_json::json!({ "error": "at least one log question must be enabled" });
		}
		state.db.set_setting("enabled_log_question_ids", &serde_json::to_string(&ids).unwrap());
	}

	if let Some(notifications) = &notifications {
		for notification in notifications {
			let title = notification["title"].as_str().unwrap_or("");
			if title == "Daily intention reminder" {
				if let Some(value) = notification["value"].as_str() {
					state.db.set_setting("reminder_time", value);
				}
				if let Some(enabled) = notification["enabled"].as_bool() {
					state.db.set_setting("reminder_enabled", if enabled { "true" } else { "false" });
				}
			}
		}
	}

	serde_json::json!({ "id": "settings" })
}

#[tauri::command]
pub fn get_ai_settings(state: State<'_, AppState>) -> serde_json::Value {
	let s = AiSettings::load(&state.db);
	// camelCase to match the frontend's field access
	serde_json::json!({
		"llmMode": s.llm_mode,
		"llmModel": s.llm_model,
		"sttModel": s.stt_model,
		"extLlmBaseUrl": s.ext_llm_base_url,
		"extLlmApiKey": s.ext_llm_api_key,
		"extLlmModel": s.ext_llm_model,
		"extSttBaseUrl": s.ext_stt_base_url,
		"extSttApiKey": s.ext_stt_api_key,
		"extSttModel": s.ext_stt_model,
	})
}

#[tauri::command]
pub fn save_ai_settings(
	app: tauri::AppHandle,
	state: State<'_, AppState>,
	ai: serde_json::Value,
) -> Result<(), String> {
	let mut settings = AiSettings::load(&state.db);
	let get_str = |key: &str| ai[key].as_str().map(|s| s.to_string());

	if let Some(v) = get_str("llmMode") {
		settings.llm_mode = v;
	}
	if let Some(v) = get_str("llmModel") {
		settings.llm_model = v;
	}
	if let Some(v) = get_str("sttModel") {
		settings.stt_model = v;
	}
	for key in [
		"extLlmBaseUrl",
		"extLlmApiKey",
		"extLlmModel",
		"extSttBaseUrl",
		"extSttApiKey",
		"extSttModel",
	] {
		if let Some(v) = get_str(key) {
			match key {
				"extLlmBaseUrl" => settings.ext_llm_base_url = v,
				"extLlmApiKey" => settings.ext_llm_api_key = v,
				"extLlmModel" => settings.ext_llm_model = v,
				"extSttBaseUrl" => settings.ext_stt_base_url = v,
				"extSttApiKey" => settings.ext_stt_api_key = v,
				"extSttModel" => settings.ext_stt_model = v,
				_ => {}
			}
		}
	}
	settings.save(&state.db);

	// Activate models that are ready to go with the new settings.
	crate::spawn_model_loader(app.clone(), settings);
	Ok(())
}

#[tauri::command]
pub async fn test_llm_endpoint(state: State<'_, AppState>) -> Result<String, String> {
	let settings = AiSettings::load(&state.db);
	if settings.ext_llm_base_url.is_empty() {
		return Err("no external LLM endpoint configured".into());
	}
	let client = crate::llm::ExternalLlm::new(
		&settings.ext_llm_base_url,
		&settings.ext_llm_api_key,
		if settings.ext_llm_model.is_empty() { "default" } else { &settings.ext_llm_model },
	);
	let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
	let mut got_any = false;
	let output = client
		.generate(
			"You are a helpful assistant.",
			&[crate::types::ChatMessage { role: "user".into(), content: "Say OK".into() }],
			&cancel,
			|_| got_any = true,
		)
		.await?;
	if output.trim().is_empty() && !got_any {
		return Err("endpoint responded with an empty reply".into());
	}
	Ok(format!("endpoint OK ({})", settings.ext_llm_base_url))
}

#[tauri::command]
pub async fn test_stt_endpoint(state: State<'_, AppState>) -> Result<String, String> {
	let settings = AiSettings::load(&state.db);
	if settings.ext_stt_base_url.is_empty() {
		return Err("no external STT endpoint configured".into());
	}
	// 0.5 s of silence is enough to validate auth + routing.
	let silence = vec![0i16; 8000];
	let wav = encode_tiny_wav(&silence, 16000);
	let _ = crate::stt::transcribe_external(
		&settings.ext_stt_base_url,
		&settings.ext_stt_api_key,
		&settings.ext_stt_model,
		wav,
	)
	.await?;
	Ok(format!("endpoint OK ({})", settings.ext_stt_base_url))
}

fn encode_tiny_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
	let mut out: Vec<u8> = Vec::new();
	out.extend_from_slice(b"RIFF");
	let data_len = (samples.len() * 2) as u32;
	out.extend_from_slice(&((36 + data_len)).to_le_bytes());
	out.extend_from_slice(b"WAVE");
	out.extend_from_slice(b"fmt ");
	out.extend_from_slice(&16u32.to_le_bytes());
	out.extend_from_slice(&1u16.to_le_bytes()); // PCM
	out.extend_from_slice(&1u16.to_le_bytes()); // mono
	out.extend_from_slice(&sample_rate.to_le_bytes());
	out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
	out.extend_from_slice(&2u16.to_le_bytes()); // block align
	out.extend_from_slice(&16u16.to_le_bytes()); // bits
	out.extend_from_slice(b"data");
	out.extend_from_slice(&data_len.to_le_bytes());
	for sample in samples {
		out.extend_from_slice(&sample.to_le_bytes());
	}
	out
}
