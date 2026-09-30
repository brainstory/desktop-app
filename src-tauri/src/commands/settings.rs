use tauri::{Manager, State};

use crate::db::DEFAULT_LOG_QUESTIONS;
use crate::keys;
use crate::types::{
	AppPresence, LogSettingsItem, NotificationSettingsItem, UserSettings, UserSettingsUser,
};
use crate::AppState;

#[tauri::command]
pub async fn get_user_settings(state: State<'_, AppState>) -> Result<UserSettings, String> {
	let name = state
		.db
		.get_setting(keys::setting::USER_NAME)
		.filter(|s| !s.is_empty());
	let timezone = state
		.db
		.get_setting(keys::setting::USER_TIMEZONE)
		.filter(|s| !s.is_empty());
	let reminder_enabled = state
		.db
		.get_setting(keys::setting::REMINDER_ENABLED)
		.map(|v| v == "true")
		.unwrap_or(false);
	let reminder_time = state
		.db
		.get_setting(keys::setting::REMINDER_TIME)
		.filter(|s| !s.is_empty())
		.unwrap_or_else(|| "09:00".into());
	let presence = AppPresence {
		dock: state
			.db
			.get_setting(keys::setting::SHOW_IN_DOCK)
			.map(|v| v == "true")
			.unwrap_or(true),
		tray: state
			.db
			.get_setting(keys::setting::SHOW_IN_TRAY)
			.map(|v| v == "true")
			.unwrap_or(true),
	};

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

	Ok(UserSettings {
		user: UserSettingsUser { name, timezone },
		log,
		notifications,
		presence,
	})
}

/// Strictly parse a "HH:MM" reminder time. Single source of truth for
/// the format: the settings form validates with it and the reminder
/// loop parses with it, so the two can never disagree on what counts as
/// a valid time.
pub(crate) fn parse_hhmm(value: &str) -> Option<(u32, u32)> {
	let mut parts = value.split(':');
	let (Some(hours), Some(minutes), None) = (parts.next(), parts.next(), parts.next()) else {
		return None;
	};
	match (hours.parse::<u32>(), minutes.parse::<u32>()) {
		(Ok(h), Ok(m)) if h <= 23 && m <= 59 && minutes.len() == 2 => Some((h, m)),
		_ => None,
	}
}

fn valid_reminder_time(value: &str) -> bool {
	parse_hhmm(value).is_some()
}

#[tauri::command]
pub async fn save_user_settings(
	state: State<'_, AppState>,
	user: Option<serde_json::Value>,
	enabled_log_question_ids: Option<Vec<i64>>,
	notifications: Option<Vec<serde_json::Value>>,
) -> Result<serde_json::Value, String> {
	// Validate everything first and collect the writes, so the settings
	// land in ONE transaction (a failure can no longer persist half the
	// form) and side effects only run once persisted.
	let mut kv: Vec<(&str, String)> = Vec::new();
	if let Some(user) = &user {
		if let Some(name) = user["name"].as_str() {
			kv.push((keys::setting::USER_NAME, name.to_string()));
		}
		if let Some(timezone) = user["timezone"].as_str() {
			kv.push((keys::setting::USER_TIMEZONE, timezone.to_string()));
		}
	}

	if let Some(ids) = &enabled_log_question_ids {
		let mut ids: Vec<i64> = ids
			.iter()
			.copied()
			.filter(|id| DEFAULT_LOG_QUESTIONS.iter().any(|(qid, _, _)| qid == id))
			.collect();
		ids.sort();
		ids.dedup();
		if ids.is_empty() {
			return Err("at least one log question must be enabled".into());
		}
		kv.push((
			keys::setting::ENABLED_LOG_QUESTION_IDS,
			serde_json::to_string(&ids).expect("serializing Vec<i64> cannot fail"),
		));
	}

	let mut reminder_enabled_after: Option<bool> = None;
	let mut time_changed = false;
	if let Some(notifications) = &notifications {
		for notification in notifications {
			let title = notification["title"].as_str().unwrap_or("");
			if title == "Daily intention reminder" {
				if let Some(value) = notification["value"].as_str() {
					if !valid_reminder_time(value) {
						return Err(format!("invalid reminder time '{value}' (expected HH:MM)"));
					}
					kv.push((keys::setting::REMINDER_TIME, value.to_string()));
					time_changed = true;
				}
				if let Some(enabled) = notification["enabled"].as_bool() {
					kv.push((
						keys::setting::REMINDER_ENABLED,
						if enabled {
							"true".into()
						} else {
							"false".into()
						},
					));
					reminder_enabled_after = Some(enabled);
				}
			}
		}
	}

	if !kv.is_empty() {
		state.db.set_settings(&kv)?;
	}
	if let Some(enabled) = reminder_enabled_after {
		// keep the tray menu checkmark in sync with the persisted setting
		crate::sync_tray_reminder_check(enabled);
	}
	if reminder_enabled_after.is_some() || time_changed {
		// wake the scheduler so a new time/state applies immediately
		crate::reminders::REMINDER_SETTINGS_CHANGED.notify_waiters();
	}

	Ok(serde_json::json!({ "id": "settings" }))
}

#[tauri::command]
pub async fn get_ai_settings(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
	// AiSettings::load does keychain reads; keep them off the main thread
	tauri::async_runtime::spawn_blocking(move || {
		let state = app.state::<AppState>();
		let s = state.ai_settings();
		// Secrets are never echoed to the webview: the form gets a "is one
		// stored" flag plus a masked hint; saving absent/null keeps the stored
		// value and an empty string clears it.
		let masked = |value: &str| -> (bool, Option<String>) {
			if value.is_empty() {
				return (false, None);
			}
			let tail: String = value
				.chars()
				.skip(value.chars().count().saturating_sub(4))
				.collect();
			(true, Some(format!("••••{tail}")))
		};
		let (hf_set, hf_hint) = masked(&s.hf_token);
		let (llm_key_set, llm_key_hint) = masked(&s.ext_llm_api_key);
		let (stt_key_set, stt_key_hint) = masked(&s.ext_stt_api_key);
		// camelCase to match the frontend's field access
		Ok(serde_json::json!({
			"llmMode": s.llm_mode.as_str(),
			"llmModel": s.llm_model,
			"sttModel": s.stt_model,
			"sttEngine": s.stt_engine.as_str(),
			"sttLanguage": s.stt_language,
			"hfTokenSet": hf_set,
			"hfTokenHint": hf_hint,
			"extLlmBaseUrl": s.ext_llm_base_url,
			"extLlmApiKeySet": llm_key_set,
			"extLlmApiKeyHint": llm_key_hint,
			"extLlmModel": s.ext_llm_model,
			"extSttBaseUrl": s.ext_stt_base_url,
			"extSttApiKeySet": stt_key_set,
			"extSttApiKeyHint": stt_key_hint,
			"extSttModel": s.ext_stt_model,
		}))
	})
	.await
	.map_err(|e| format!("ai settings task failed: {e}"))?
}

#[tauri::command]
pub async fn save_ai_settings(app: tauri::AppHandle, ai: serde_json::Value) -> Result<(), String> {
	// keychain reads/writes plus the settings-row write are blocking work
	tauri::async_runtime::spawn_blocking(move || {
		let state = app.state::<AppState>();
		let mut settings = state.ai_settings();
		settings.apply_updates(&ai)?;
		state.save_ai_settings(&settings)?;

		// Activate models that are ready to go with the new settings.
		crate::spawn_model_loader(app.clone(), settings);
		Ok(())
	})
	.await
	.map_err(|e| format!("save ai settings task failed: {e}"))?
}

#[tauri::command]
pub async fn test_llm_endpoint(state: State<'_, AppState>) -> Result<String, String> {
	let settings = state.ai_settings();
	if settings.ext_llm_base_url.is_empty() {
		return Err("no external LLM endpoint configured".into());
	}
	if !settings.ext_llm_base_url.starts_with("http://")
		&& !settings.ext_llm_base_url.starts_with("https://")
	{
		return Err(format!(
			"invalid endpoint URL '{}' (include http:// or https://)",
			settings.ext_llm_base_url
		));
	}
	let client = crate::llm::ExternalLlm::new(
		&settings.ext_llm_base_url,
		&settings.ext_llm_api_key,
		if settings.ext_llm_model.is_empty() {
			"default"
		} else {
			&settings.ext_llm_model
		},
	)?;
	let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
	let mut got_any = false;
	let (output, _) = client
		.generate(
			"You are a helpful assistant.",
			&[crate::types::ChatMessage {
				role: "user".into(),
				content: "Say OK".into(),
			}],
			&cancel,
			16,
			|_| got_any = true,
		)
		.await?;
	if output.trim().is_empty() && !got_any {
		return Err("endpoint responded with an empty reply".into());
	}
	if !settings.uses_external_llm() {
		return Ok(format!(
			"endpoint OK ({}) - but chats still use the local model. Turn on 'Use external LLM endpoint' above to route chats here.",
			settings.ext_llm_base_url
		));
	}
	Ok(format!("endpoint OK ({})", settings.ext_llm_base_url))
}

#[tauri::command]
pub async fn test_stt_endpoint(state: State<'_, AppState>) -> Result<String, String> {
	let settings = state.ai_settings();
	if settings.ext_stt_base_url.is_empty() {
		return Err("no external STT endpoint configured".into());
	}
	// 0.5 s of silence is enough to validate auth + routing.
	let silence = vec![0.0f32; 8000];
	let wav = crate::voice::encode_wav_16k(&silence)?;
	let _ = crate::stt::transcribe_external(
		&settings.ext_stt_base_url,
		&settings.ext_stt_api_key,
		&settings.ext_stt_model,
		wav,
	)
	.await?;
	Ok(format!("endpoint OK ({})", settings.ext_stt_base_url))
}

/// Toggle dock / menu-bar (tray) icon visibility. Applied immediately and
/// persisted for future launches.
#[tauri::command]
pub fn set_app_presence(
	app: tauri::AppHandle,
	state: State<'_, AppState>,
	dock: bool,
	tray: bool,
) -> Result<(), String> {
	if !dock && !tray {
		return Err(
			"Brainstory can't be hidden from both the Dock and the menu bar - keep at least one visible."
				.into(),
		);
	}
	state.db.set_setting(
		keys::setting::SHOW_IN_DOCK,
		if dock { "true" } else { "false" },
	)?;
	state.db.set_setting(
		keys::setting::SHOW_IN_TRAY,
		if tray { "true" } else { "false" },
	)?;
	crate::apply_presence(app.app_handle(), dock, tray);
	Ok(())
}
