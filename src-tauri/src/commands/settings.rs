use tauri::{Manager, State};

use crate::db::DEFAULT_LOG_QUESTIONS;
use crate::keys;
use crate::types::{
	AppPresence, LogSettingsItem, NotificationSettingsItem, UpdatesSettings, UserSettings,
	UserSettingsUser,
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
		updates: UpdatesSettings {
			enabled: updates_enabled(&state),
		},
	})
}

/// Strictly parse a "HH:MM" reminder time, also accepting the
/// "HH:MM:SS" the settings page's time picker sends (and that early
/// builds stored verbatim); seconds are validated, then ignored. Single
/// source of truth for the format: the settings form validates with it
/// and the reminder loop parses with it, so the two can never disagree
/// on what counts as a valid time.
pub(crate) fn parse_hhmm(value: &str) -> Option<(u32, u32)> {
	// exactly two ASCII digits: u32::from_str alone would also take "+9"
	let two_digits = |field: &str, max: u32| -> Option<u32> {
		if field.len() != 2 || !field.bytes().all(|b| b.is_ascii_digit()) {
			return None;
		}
		field.parse::<u32>().ok().filter(|v| *v <= max)
	};
	let mut parts = value.split(':');
	let (Some(hours), Some(minutes)) = (parts.next(), parts.next()) else {
		return None;
	};
	match (parts.next(), parts.next()) {
		(None, _) => {}
		(Some(seconds), None) => {
			two_digits(seconds, 59)?;
		}
		_ => return None,
	}
	Some((two_digits(hours, 23)?, two_digits(minutes, 59)?))
}

/// The canonical stored form of a valid reminder time ("HH:MM").
fn normalize_reminder_time(value: &str) -> Option<String> {
	parse_hhmm(value).map(|(h, m)| format!("{h:02}:{m:02}"))
}

/// The validated writes of one save_user_settings call, plus what changed
/// for the side effects that run after persisting.
#[derive(Debug, Default)]
struct UserSettingsUpdate {
	kv: Vec<(&'static str, String)>,
	reminder_enabled_after: Option<bool>,
	time_changed: bool,
	timezone_changed: bool,
}

/// Whether this save must re-arm the reminder scheduler: a reminder
/// field (enabled/time, as before) OR the timezone - the daily boundary
/// and the due instant both move with the zone.
fn wakes_scheduler(update: &UserSettingsUpdate) -> bool {
	update.reminder_enabled_after.is_some() || update.time_changed || update.timezone_changed
}

/// Validate the whole form and persist it in ONE transaction: an invalid
/// field (or a failed write) persists nothing, never half the form.
fn persist_user_settings(
	db: &crate::db::Db,
	user: Option<&serde_json::Value>,
	enabled_log_question_ids: Option<&[i64]>,
	notifications: Option<&[serde_json::Value]>,
) -> Result<UserSettingsUpdate, String> {
	let update = collect_user_settings(user, enabled_log_question_ids, notifications)?;
	if !update.kv.is_empty() {
		db.set_settings(&update.kv)?;
	}
	Ok(update)
}

fn collect_user_settings(
	user: Option<&serde_json::Value>,
	enabled_log_question_ids: Option<&[i64]>,
	notifications: Option<&[serde_json::Value]>,
) -> Result<UserSettingsUpdate, String> {
	let mut kv: Vec<(&'static str, String)> = Vec::new();
	let mut timezone_changed = false;
	if let Some(user) = user {
		if let Some(name) = user["name"].as_str() {
			kv.push((keys::setting::USER_NAME, name.to_string()));
		}
		if let Some(timezone) = user["timezone"].as_str() {
			// A non-empty zone must name a zone this build's embedded
			// tz database knows, or day boundaries and reminders would
			// silently follow the OS zone instead of the saved value.
			// Empty stays allowed (= follow the OS zone).
			if !timezone.is_empty() && crate::db::parse_zone(timezone).is_none() {
				return Err(format!(
					"invalid timezone '{timezone}' (expected an IANA zone name like Europe/Berlin, or empty to follow this computer's zone)"
				));
			}
			kv.push((keys::setting::USER_TIMEZONE, timezone.to_string()));
			timezone_changed = true;
		}
	}

	if let Some(ids) = enabled_log_question_ids {
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
	if let Some(notifications) = notifications {
		for notification in notifications {
			let title = notification["title"].as_str().unwrap_or("");
			if title == "Daily intention reminder" {
				if let Some(value) = notification["value"].as_str() {
					let Some(time) = normalize_reminder_time(value) else {
						return Err(format!("invalid reminder time '{value}' (expected HH:MM)"));
					};
					kv.push((keys::setting::REMINDER_TIME, time));
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

	Ok(UserSettingsUpdate {
		kv,
		reminder_enabled_after,
		time_changed,
		timezone_changed,
	})
}

#[tauri::command]
pub async fn save_user_settings(
	state: State<'_, AppState>,
	user: Option<serde_json::Value>,
	enabled_log_question_ids: Option<Vec<i64>>,
	notifications: Option<Vec<serde_json::Value>>,
) -> Result<serde_json::Value, String> {
	// Side effects only run once the whole form is persisted.
	let update = persist_user_settings(
		&state.db,
		user.as_ref(),
		enabled_log_question_ids.as_deref(),
		notifications.as_deref(),
	)?;
	if let Some(enabled) = update.reminder_enabled_after {
		// keep the tray menu checkmark in sync with the persisted setting
		crate::sync_tray_reminder_check(enabled);
	}
	if wakes_scheduler(&update) {
		// wake the scheduler so a new time/state/zone applies immediately
		crate::reminders::notify_settings_changed();
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
			"llmCtxTokens": s.llm_ctx_tokens,
			"sttModel": s.stt_model,
			"sttMode": s.stt_mode.as_str(),
			"sttEngine": s.stt_engine.as_str(),
			"sttLanguage": s.stt_language,
			"hfTokenSet": hf_set,
			"hfTokenHint": hf_hint,
			"hfEndpoint": s.hf_endpoint,
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
		// one critical section over read-latest, apply and persist, so a
		// concurrent writer (model activation, another save) can never be
		// overwritten by a stale full snapshot
		let (settings, generation) = state.mutate_ai_settings(|s| s.apply_updates(&ai))?;

		// Activate models that are ready to go with the new settings.
		// Started only after the mutation committed, never under its
		// lock. The returned generation is the pair of the settings
		// snapshot: the loader treats any later generation as newer and
		// lets that run own the outcome.
		crate::spawn_model_loader(app.clone(), settings, generation);
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
	if !crate::models::has_http_scheme(&settings.ext_llm_base_url) {
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
	if !settings.uses_external_stt() {
		return Ok(format!(
			"endpoint OK ({}) - but transcription still runs on this computer. Turn on 'Use external STT endpoint' to send it here.",
			settings.ext_stt_base_url
		));
	}
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

/// Automatic update checks are on unless the user turned them off.
pub(crate) fn updates_enabled(state: &AppState) -> bool {
	state
		.db
		.get_setting(keys::setting::UPDATES_ENABLED)
		.map(|v| v == "true")
		.unwrap_or(true)
}

fn store_updates_enabled(state: &AppState, enabled: bool) -> Result<(), String> {
	state.db.set_setting(
		keys::setting::UPDATES_ENABLED,
		if enabled { "true" } else { "false" },
	)
}

/// Cheap read for the updater banner, which checks it before every
/// (throttled) update check.
#[tauri::command]
pub async fn get_updates_enabled(state: State<'_, AppState>) -> Result<bool, String> {
	Ok(updates_enabled(&state))
}

/// Opt in to / out of automatic update checks.
#[tauri::command]
pub async fn set_updates_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
	store_updates_enabled(&state, enabled)
}

#[cfg(test)]
mod updates_tests {
	use super::{store_updates_enabled, updates_enabled};
	use crate::AppState;

	fn state() -> (AppState, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::create_dir_all(dir.path().join("models")).expect("make models dir");
		let db = crate::db::Db::open(&dir.path().join("t.db")).expect("db");
		(AppState::new(db, dir.path().to_path_buf()), dir)
	}

	#[test]
	fn updates_are_enabled_by_default() {
		let (state, _dir) = state();
		assert!(updates_enabled(&state));
	}

	#[test]
	fn updates_enabled_round_trips() {
		let (state, _dir) = state();
		store_updates_enabled(&state, false).expect("store false");
		assert!(!updates_enabled(&state));
		store_updates_enabled(&state, true).expect("store true");
		assert!(updates_enabled(&state));
	}
}

#[cfg(test)]
mod tests {
	use super::persist_user_settings;
	use crate::keys::setting;
	use serde_json::json;

	fn temp_db() -> (crate::db::Db, std::path::PathBuf, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		let path = dir.path().join("settings.db");
		(crate::db::Db::open(&path).expect("open"), path, dir)
	}

	fn reminder(value: &str) -> Vec<serde_json::Value> {
		vec![json!({ "title": "Daily intention reminder", "value": value, "enabled": true })]
	}

	#[test]
	fn an_invalid_field_persists_nothing_from_the_batch() {
		let (db, _path, _dir) = temp_db();
		let err = persist_user_settings(
			&db,
			Some(&json!({ "name": "Ada" })),
			Some(&[1, 2]),
			Some(&reminder("25:00")),
		)
		.expect_err("an invalid reminder time rejects the whole form");
		assert!(err.contains("invalid reminder time"), "unexpected: {err}");
		for key in [
			setting::USER_NAME,
			setting::ENABLED_LOG_QUESTION_IDS,
			setting::REMINDER_TIME,
			setting::REMINDER_ENABLED,
		] {
			assert_eq!(
				db.get_setting(key),
				None,
				"{key} leaked from a rejected form"
			);
		}
	}

	#[test]
	fn a_failed_write_rolls_back_the_whole_batch() {
		let (db, path, _dir) = temp_db();
		{
			// the reminder row's write fails after the name row was written
			let conn = rusqlite::Connection::open(&path).unwrap();
			conn.execute_batch(
				"CREATE TRIGGER fail_reminder BEFORE INSERT ON settings
				 WHEN NEW.key = 'reminder_time'
				 BEGIN SELECT RAISE(ABORT, 'disk on fire'); END;",
			)
			.unwrap();
		}
		let err = persist_user_settings(
			&db,
			Some(&json!({ "name": "Ada" })),
			None,
			Some(&reminder("08:00")),
		)
		.expect_err("the failing write surfaces");
		assert!(err.contains("disk on fire"), "unexpected: {err}");
		assert_eq!(
			db.get_setting(setting::USER_NAME),
			None,
			"the name written before the failure is rolled back"
		);
	}

	#[test]
	fn the_time_pickers_hh_mm_ss_value_saves_as_hh_mm() {
		// NotificationsCard sends `${hour}:00:00`; that is the only shape
		// the settings page ever produces, so it must save
		let (db, _path, _dir) = temp_db();
		persist_user_settings(&db, None, None, Some(&reminder("14:00:00")))
			.expect("the frontend's own format is valid");
		assert_eq!(
			db.get_setting(setting::REMINDER_TIME).as_deref(),
			Some("14:00"),
			"stored normalized"
		);
		// rows saved that way by early builds keep firing at their hour
		assert_eq!(super::parse_hhmm("13:00:00"), Some((13, 0)));
		assert_eq!(super::parse_hhmm("13:00:60"), None);
		assert_eq!(super::parse_hhmm("13:00:00:00"), None);
	}

	#[test]
	fn reminder_times_need_two_plain_digits_per_field() {
		use super::parse_hhmm;
		for valid in ["00:00", "09:05", "23:59", "12:00:00"] {
			assert!(parse_hhmm(valid).is_some(), "{valid:?} must parse");
		}
		// u32::from_str accepts a leading '+', and the hour had no length
		// check: none of these is what the time picker ever produces
		for invalid in [
			"9:00", "+9:00", "+09:00", "12:+5", "09:00:+1", "009:00", "09:0", " 9:00", "",
		] {
			assert_eq!(parse_hhmm(invalid), None, "{invalid:?} must be rejected");
		}
	}

	#[test]
	fn a_valid_form_persists_every_field() {
		let (db, _path, _dir) = temp_db();
		let update = persist_user_settings(
			&db,
			Some(&json!({ "name": "Ada", "timezone": "Europe/Berlin" })),
			Some(&[2, 99, 1, 2]),
			Some(&reminder("08:00")),
		)
		.expect("valid form");
		assert_eq!(update.reminder_enabled_after, Some(true));
		assert!(update.time_changed);
		assert_eq!(db.get_setting(setting::USER_NAME).as_deref(), Some("Ada"));
		assert_eq!(
			db.get_setting(setting::ENABLED_LOG_QUESTION_IDS).as_deref(),
			Some("[1,2]"),
			"unknown ids dropped, sorted, deduped"
		);
		assert_eq!(
			db.get_setting(setting::REMINDER_TIME).as_deref(),
			Some("08:00")
		);
	}

	#[test]
	fn an_invalid_timezone_rejects_the_whole_form() {
		let (db, _path, _dir) = temp_db();
		let err = persist_user_settings(
			&db,
			Some(&json!({ "name": "Ada", "timezone": "Mars/Olympus_Mons" })),
			None,
			Some(&reminder("08:00")),
		)
		.expect_err("a non-empty timezone must parse as an IANA zone name");
		assert!(err.contains("invalid timezone"), "unexpected: {err}");
		for key in [
			setting::USER_NAME,
			setting::USER_TIMEZONE,
			setting::REMINDER_TIME,
		] {
			assert_eq!(
				db.get_setting(key),
				None,
				"{key} leaked from a rejected form"
			);
		}
	}

	#[test]
	fn an_empty_timezone_stays_allowed_as_the_os_local_fallback() {
		let (db, _path, _dir) = temp_db();
		persist_user_settings(
			&db,
			Some(&json!({ "name": "Ada", "timezone": "" })),
			None,
			None,
		)
		.expect("empty means 'follow the OS zone', not an error");
		assert_eq!(db.get_setting(setting::USER_NAME).as_deref(), Some("Ada"));
		assert_eq!(
			db.get_setting(setting::USER_TIMEZONE).as_deref(),
			Some(""),
			"the empty value persists (no OS-local zone is substituted here)"
		);
	}

	#[test]
	fn a_zone_save_switches_the_active_zone_without_restart() {
		let (db, _path, _dir) = temp_db();
		persist_user_settings(&db, Some(&json!({ "timezone": "Asia/Tokyo" })), None, None)
			.expect("save Tokyo");
		assert_eq!(
			db.active_zone().map(|tz| tz.name().to_string()),
			Some("Asia/Tokyo".to_string()),
			"the zone takes effect on the open database, not after a restart"
		);
		persist_user_settings(
			&db,
			Some(&json!({ "timezone": "America/New_York" })),
			None,
			None,
		)
		.expect("switch zone");
		assert_eq!(
			db.active_zone().map(|tz| tz.name().to_string()),
			Some("America/New_York".to_string()),
			"a later save replaces the earlier zone"
		);
	}

	#[test]
	fn a_timezone_only_save_re_arms_the_scheduler() {
		let zone_only =
			super::collect_user_settings(Some(&json!({ "timezone": "Asia/Tokyo" })), None, None)
				.expect("valid zone-only form");
		assert!(
			super::wakes_scheduler(&zone_only),
			"the due instant moves with the zone"
		);
		let nothing = super::collect_user_settings(Some(&json!({ "name": "Ada" })), None, None)
			.expect("name-only form");
		assert!(
			!super::wakes_scheduler(&nothing),
			"a name-only save must not wake the loop"
		);
	}
}
