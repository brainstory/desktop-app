use tauri::Manager;

use crate::commands::run_blocking;
use crate::db::{DEFAULT_LOG_QUESTIONS, SURVEY_QUESTIONS};
use crate::types::{DailyStatus, IdeaItem, IdeaType, LogAnswerItem, LogQuestionItem, UserData};
use crate::AppState;

fn idea_list(ideas: Vec<IdeaItem>) -> serde_json::Value {
	serde_json::json!({ "ideas": ideas })
}

/// Upper bounds on webview-supplied payloads: normal use is orders of
/// magnitude below these, so anything larger is a bug or a compromised
/// webview trying to exhaust memory or disk.
const MAX_RESULT_BYTES: usize = 2 * 1024 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 4 * 1024 * 1024;
const MAX_STRUCTURED_BYTES: usize = 1024 * 1024;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_NOTE_BYTES: usize = 16 * 1024;
const MAX_SURVEY_BYTES: usize = 64 * 1024;

fn ensure_within(field: &str, value: &str, max: usize) -> Result<(), String> {
	if value.len() > max {
		return Err(format!(
			"{field} is too large ({} bytes, at most {max} accepted)",
			value.len()
		));
	}
	Ok(())
}

#[tauri::command]
pub async fn get_user(app: tauri::AppHandle) -> Result<UserData, String> {
	run_blocking("get user", move || {
		let state = app.state::<AppState>();
		let name = state
			.db
			.get_setting(crate::keys::setting::USER_NAME)
			.filter(|s| !s.is_empty());
		let timezone = state
			.db
			.get_setting(crate::keys::setting::USER_TIMEZONE)
			.filter(|s| !s.is_empty());
		let created_at = state
			.db
			.get_setting(crate::keys::setting::CREATED_AT)
			.unwrap_or_else(|| "1970-01-01T00:00:00".into());
		Ok(UserData {
			name,
			timezone,
			created_at,
		})
	})
	.await
}

#[tauri::command]
pub async fn get_daily_status(app: tauri::AppHandle) -> Result<DailyStatus, String> {
	run_blocking("daily status", move || {
		let state = app.state::<AppState>();
		Ok(state.db.get_daily_status())
	})
	.await
}

/// Open the app's data folder (database, models metadata) in the
/// system file manager.
#[tauri::command]
pub fn open_data_folder(app: tauri::AppHandle) -> Result<(), String> {
	let dir = app.state::<AppState>().data_dir.clone();
	open_in_file_manager(&app, &dir)
}

/// Reveal the newest log file in the system file manager.
#[tauri::command]
pub fn open_log_folder(app: tauri::AppHandle) -> Result<(), String> {
	use tauri::Manager;
	let dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
	// the newest brainstory log file (rotation may have renamed it)
	let log_file = std::fs::read_dir(&dir)
		.map_err(|e| e.to_string())?
		.filter_map(|entry| entry.ok())
		.filter(|entry| {
			entry
				.file_name()
				.to_string_lossy()
				.starts_with("brainstory")
		})
		.max_by_key(|entry| {
			entry
				.metadata()
				.ok()
				.and_then(|meta| meta.modified().ok())
				.map(|time| {
					time.duration_since(std::time::UNIX_EPOCH)
						.map(|d| d.as_secs())
						.unwrap_or(0)
				})
				.unwrap_or(0)
		})
		.map(|entry| entry.path());
	match log_file {
		Some(file) => reveal_in_file_manager(&app, &file),
		None => open_in_file_manager(&app, &dir),
	}
}

fn open_in_file_manager(app: &tauri::AppHandle, path: &std::path::Path) -> Result<(), String> {
	use tauri_plugin_opener::OpenerExt;
	app.opener()
		.open_path(path.to_string_lossy().to_string(), None::<&str>)
		.map_err(|e| e.to_string())
}

fn reveal_in_file_manager(app: &tauri::AppHandle, path: &std::path::Path) -> Result<(), String> {
	use tauri_plugin_opener::OpenerExt;
	app.opener()
		.reveal_item_in_dir(path.to_string_lossy().to_string())
		.map_err(|e| e.to_string())
}

/// A diagnostics bundle for support: app version, OS, the active engine
/// statuses, free disk space and the tail of the log. Nothing leaves the
/// device - the caller copies it to the clipboard.
#[tauri::command]
pub fn copy_diagnostics(app: tauri::AppHandle) -> Result<String, String> {
	use tauri::Manager;
	let state = app.state::<AppState>();
	let mut report = String::new();
	report.push_str(&format!(
		"Brainstory {} ({})\n",
		env!("CARGO_PKG_VERSION"),
		std::env::consts::OS
	));
	let llm = state.llm_status.lock().unwrap_or_else(|e| e.into_inner());
	report.push_str(&format!(
		"LLM: {:?} {}\n",
		llm.state,
		llm.model_id.as_deref().unwrap_or("-")
	));
	drop(llm);
	let stt = state.stt_status.lock().unwrap_or_else(|e| e.into_inner());
	report.push_str(&format!(
		"STT: {:?} {}\n",
		stt.state,
		stt.model_id.as_deref().unwrap_or("-")
	));
	drop(stt);
	if let Ok(free) = fs4::available_space(&state.data_dir) {
		report.push_str(&format!("free disk: {} MB\n", free / (1024 * 1024)));
	}
	if let Ok(dir) = app.path().app_log_dir() {
		let log_file = std::fs::read_dir(&dir).ok().and_then(|entries| {
			entries
				.filter_map(|entry| entry.ok())
				.filter(|entry| {
					entry
						.file_name()
						.to_string_lossy()
						.starts_with("brainstory")
				})
				.max_by_key(|entry| {
					entry
						.metadata()
						.ok()
						.and_then(|meta| meta.modified().ok())
						.map(|time| {
							time.duration_since(std::time::UNIX_EPOCH)
								.map(|d| d.as_secs())
								.unwrap_or(0)
						})
						.unwrap_or(0)
				})
				.map(|entry| entry.path())
		});
		if let Some(file) = log_file {
			if let Ok(contents) = std::fs::read_to_string(&file) {
				let tail: Vec<&str> = contents.lines().rev().take(50).collect();
				report.push_str("--- last log lines ---\n");
				for line in tail.iter().rev() {
					report.push_str(line);
					report.push('\n');
				}
			}
		}
	}
	Ok(report)
}

/// Link an idea to a daily log that was submitted after the idea was
/// created (the daily-intent session's backfill path).
#[tauri::command]
pub async fn link_idea_log(
	app: tauri::AppHandle,
	idea_id: String,
	log_id: String,
) -> Result<(), String> {
	if log_id.trim().is_empty() {
		return Err("log id must not be empty".into());
	}
	run_blocking("link idea log", move || {
		let state = app.state::<AppState>();
		state.db.link_idea_log(&idea_id, &log_id)
	})
	.await
}

#[tauri::command]
pub async fn get_all_ideas(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
	run_blocking("list ideas", move || {
		let state = app.state::<AppState>();
		Ok(idea_list(state.db.list_ideas()?))
	})
	.await
}

#[tauri::command]
pub async fn get_idea(app: tauri::AppHandle, idea_id: String) -> Result<IdeaItem, String> {
	run_blocking("get idea", move || {
		let state = app.state::<AppState>();
		match state.db.get_idea(&idea_id)? {
			Some(idea) => Ok(idea),
			None => Err(format!("idea {idea_id} not found")),
		}
	})
	.await
}

#[tauri::command]
pub async fn get_idea_children(
	app: tauri::AppHandle,
	idea_id: String,
) -> Result<serde_json::Value, String> {
	run_blocking("get idea children", move || {
		let state = app.state::<AppState>();
		Ok(idea_list(state.db.get_idea_children(&idea_id)?))
	})
	.await
}

#[tauri::command]
pub async fn create_idea(
	app: tauri::AppHandle,
	result: Option<String>,
	transcript: Option<Vec<crate::types::ChatMessage>>,
	parent_idea_id: Option<String>,
	idea_metadata: Option<serde_json::Value>,
	idea_type: Option<IdeaType>,
	log_id: Option<String>,
) -> Result<serde_json::Value, String> {
	run_blocking("create idea", move || {
		let state = app.state::<AppState>();
		state.refuse_if_shutting_down()?;
		state.refuse_if_shutting_down()?;
		let id = uuid::Uuid::new_v4().to_string();
		let idea_type = idea_type.unwrap_or_default();
		let result = result.unwrap_or_default();
		let transcript = transcript.unwrap_or_default();
		ensure_within("result", &result, MAX_RESULT_BYTES)?;
		ensure_within(
			"transcript",
			&serde_json::to_string(&transcript).unwrap_or_default(),
			MAX_TRANSCRIPT_BYTES,
		)?;
		let metadata = idea_metadata.unwrap_or_else(|| serde_json::json!({}));
		ensure_within("ideaMetadata", &metadata.to_string(), MAX_METADATA_BYTES)?;

		// (The parent's existence is verified inside insert_idea's
		// transaction, so a concurrent delete can't create an orphan.)

		let mut title = String::new();
		if !result.is_empty() {
			title = title_from_result(&result);
		}
		if idea_type == IdeaType::DailyIntent {
			state
				.db
				.create_daily_intent_idea(&id, &title, &result, &transcript, &metadata)?;
		} else {
			state.db.insert_idea(crate::db::NewIdea {
				id: &id,
				title: &title,
				idea_type,
				result: &result,
				structured_result: None,
				transcript: &transcript,
				metadata: &metadata,
				parent_idea_id: parent_idea_id.as_deref(),
				log_id: log_id.as_deref(),
				..Default::default()
			})?;
		}

		Ok(serde_json::json!({ "id": id }))
	})
	.await
}

#[tauri::command]
pub async fn update_idea(
	app: tauri::AppHandle,
	id: String,
	title: Option<String>,
	result: Option<String>,
	transcript: Option<Vec<crate::types::ChatMessage>>,
	structured_result: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
	run_blocking("update idea", move || {
		let state = app.state::<AppState>();
		state.refuse_if_shutting_down()?;
		state.refuse_if_shutting_down()?;
		if let Some(r) = &result {
			ensure_within("result", r, MAX_RESULT_BYTES)?;
		}
		if let Some(t) = &transcript {
			ensure_within(
				"transcript",
				&serde_json::to_string(t).unwrap_or_default(),
				MAX_TRANSCRIPT_BYTES,
			)?;
		}
		if let Some(s) = &structured_result {
			ensure_within("structuredResult", &s.to_string(), MAX_STRUCTURED_BYTES)?;
		}
		// One lightweight read for the derived title, instead of parsing
		// the whole transcript; not-found is enforced inside the update
		// transaction (a concurrent delete can no longer slip past the
		// check).
		let existing_title = state.db.get_idea_title(&id)?;
		let mut derived_title = title;
		if derived_title.is_none() {
			if let Some(r) = &result {
				if !r.is_empty()
					&& existing_title
						.as_deref()
						.unwrap_or_default()
						.trim()
						.is_empty()
				{
					derived_title = Some(title_from_result(r));
				}
			}
		}
		// The daily-intent completion (when a nonempty result finishes an
		// idea that is today's intent) rides the same transaction inside
		// update_idea, so a failure there can't leave the result saved
		// with the day still incomplete.
		state.db.update_idea(
			&id,
			derived_title.as_deref(),
			result.as_deref(),
			transcript.as_deref(),
			structured_result.as_ref(),
		)?;
		Ok(serde_json::json!({ "id": id }))
	})
	.await
}

#[tauri::command]
pub async fn mark_idea_read(
	app: tauri::AppHandle,
	idea_id: String,
) -> Result<serde_json::Value, String> {
	run_blocking("mark idea read", move || {
		let state = app.state::<AppState>();
		state.db.mark_idea_read(&idea_id)?;
		Ok(serde_json::json!({ "id": idea_id }))
	})
	.await
}

#[tauri::command]
pub async fn delete_idea(
	app: tauri::AppHandle,
	idea_id: String,
) -> Result<serde_json::Value, String> {
	run_blocking("delete idea", move || {
		let state = app.state::<AppState>();
		let deleted = state.db.delete_idea(&idea_id)?;
		Ok(serde_json::json!({ "id": idea_id, "deleted": deleted }))
	})
	.await
}

/// Add a user note to an idea's writeup. Notes live in the same markdown
/// result as the writeup (under a "## Notes" section) and, structured,
/// in the idea's metadata - so the Summary tab and exports include them
/// while the note list stays editable.
#[tauri::command]
pub async fn add_idea_note(
	app: tauri::AppHandle,
	idea_id: String,
	note: String,
) -> Result<serde_json::Value, String> {
	run_blocking("add idea note", move || {
		let state = app.state::<AppState>();
		let trimmed = note.trim();
		if trimmed.is_empty() {
			return Err("the note is empty".into());
		}
		ensure_within("note", trimmed, MAX_NOTE_BYTES)?;
		state.db.add_idea_note(&idea_id, trimmed)?;
		Ok(serde_json::json!({ "id": idea_id }))
	})
	.await
}

/// Replace one note's text (the note list's Edit action).
#[tauri::command]
pub async fn update_idea_note(
	app: tauri::AppHandle,
	idea_id: String,
	note_id: String,
	note: String,
) -> Result<serde_json::Value, String> {
	run_blocking("update idea note", move || {
		let state = app.state::<AppState>();
		let trimmed = note.trim();
		if trimmed.is_empty() {
			return Err("the note is empty".into());
		}
		ensure_within("note", trimmed, MAX_NOTE_BYTES)?;
		state.db.update_idea_note(&idea_id, &note_id, trimmed)?;
		Ok(serde_json::json!({ "id": idea_id, "noteId": note_id }))
	})
	.await
}

/// Delete one note (the note list's Remove action).
#[tauri::command]
pub async fn delete_idea_note(
	app: tauri::AppHandle,
	idea_id: String,
	note_id: String,
) -> Result<serde_json::Value, String> {
	run_blocking("delete idea note", move || {
		let state = app.state::<AppState>();
		state.db.delete_idea_note(&idea_id, &note_id)?;
		Ok(serde_json::json!({ "id": idea_id, "noteId": note_id }))
	})
	.await
}

#[tauri::command]
pub async fn get_log_questions(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
	run_blocking("get log questions", move || {
		let state = app.state::<AppState>();
		let enabled_ids = enabled_log_ids(&state);
		let answers = state.db.get_log_answers_today();
		let log: Vec<LogQuestionItem> = DEFAULT_LOG_QUESTIONS
			.iter()
			.map(|(id, text, label)| LogQuestionItem {
				id: *id,
				text: text.to_string(),
				label: label.to_string(),
				value: answers
					.as_ref()
					.and_then(|a| a.iter().find(|item| item.id == *id).map(|item| item.value)),
			})
			.filter(|q| enabled_ids.contains(&q.id))
			.collect();
		Ok(serde_json::json!({ "log": log }))
	})
	.await
}

#[tauri::command]
pub async fn submit_log(
	app: tauri::AppHandle,
	log: Vec<LogAnswerItem>,
) -> Result<serde_json::Value, String> {
	run_blocking("submit log", move || {
		let state = app.state::<AppState>();
		ensure_within(
			"log",
			&serde_json::to_string(&log).unwrap_or_default(),
			MAX_METADATA_BYTES,
		)?;
		let id = uuid::Uuid::new_v4().to_string();
		state.db.insert_log(&id, &log)?;
		Ok(serde_json::json!({ "id": id }))
	})
	.await
}

/// Not called by the desktop UI (kept from the web-app API surface for
/// future parity; the reflection survey flow has no UI here yet).
#[tauri::command]
pub fn get_survey_fields() -> serde_json::Value {
	serde_json::json!({ "ids": SURVEY_QUESTIONS, "range": 5 })
}

#[tauri::command]
pub async fn submit_survey(
	app: tauri::AppHandle,
	survey: serde_json::Value,
	idea_id: Option<String>,
) -> Result<serde_json::Value, String> {
	run_blocking("submit survey", move || {
		let state = app.state::<AppState>();
		ensure_within("survey", &survey.to_string(), MAX_SURVEY_BYTES)?;
		let id = uuid::Uuid::new_v4().to_string();
		state.db.insert_survey(&id, idea_id.as_deref(), &survey)?;
		Ok(serde_json::json!({ "id": id }))
	})
	.await
}

#[tauri::command]
pub fn get_notifications() -> serde_json::Value {
	// Sharing (and therefore share notifications) is removed in the desktop app.
	serde_json::json!({ "notifications": [] })
}

pub fn enabled_log_ids(state: &AppState) -> Vec<i64> {
	state
		.db
		.get_setting(crate::keys::setting::ENABLED_LOG_QUESTION_IDS)
		.and_then(|s| serde_json::from_str::<Vec<i64>>(&s).ok())
		.map(|ids| {
			// ignore ids that don't correspond to a known question
			ids.into_iter()
				.filter(|id| DEFAULT_LOG_QUESTIONS.iter().any(|(qid, _, _)| qid == id))
				.collect::<Vec<_>>()
		})
		.filter(|ids| !ids.is_empty())
		.unwrap_or_else(|| DEFAULT_LOG_QUESTIONS.iter().map(|(id, _, _)| *id).collect())
}

fn title_from_result(result: &str) -> String {
	for line in result.lines() {
		let trimmed = line.trim();
		if let Some(stripped) = trimmed.strip_prefix("#") {
			let title = stripped.trim_start_matches('#').trim();
			if !title.is_empty() {
				return title.to_string();
			}
		}
	}
	result
		.lines()
		.next()
		.unwrap_or("")
		.trim()
		.chars()
		.take(60)
		.collect()
}

#[cfg(test)]
mod tests {
	use super::{enabled_log_ids, ensure_within, title_from_result};
	use crate::AppState;

	fn state() -> (AppState, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::create_dir_all(dir.path().join("models")).expect("make models dir");
		let db = crate::db::Db::open(&dir.path().join("t.db")).expect("db");
		(AppState::new(db, dir.path().to_path_buf()), dir)
	}

	#[test]
	fn enabled_log_ids_filters_unknown_and_falls_back_to_all() {
		let (state, _dir) = state();
		// no setting yet: every default question is enabled
		assert_eq!(
			enabled_log_ids(&state),
			crate::db::DEFAULT_LOG_QUESTIONS
				.iter()
				.map(|(id, _, _)| *id)
				.collect::<Vec<_>>()
		);
		// garbage / unknown ids are dropped, real ones kept in stored order
		state
			.db
			.set_setting(crate::keys::setting::ENABLED_LOG_QUESTION_IDS, "[2, 99, 1]")
			.expect("set");
		assert_eq!(enabled_log_ids(&state), vec![2, 1]);
		// an empty (or all-unknown) selection falls back to all rather
		// than disabling the daily log entirely
		state
			.db
			.set_setting(crate::keys::setting::ENABLED_LOG_QUESTION_IDS, "[99]")
			.expect("set");
		assert_eq!(
			enabled_log_ids(&state),
			crate::db::DEFAULT_LOG_QUESTIONS
				.iter()
				.map(|(id, _, _)| *id)
				.collect::<Vec<_>>()
		);
	}

	#[test]
	fn title_from_result_prefers_heading_then_first_line_60_chars() {
		assert_eq!(
			title_from_result("## The Heading\n\nbody"),
			"The Heading",
			"first heading wins"
		);
		assert_eq!(title_from_result("no heading\nsecond line"), "no heading");
		let sixty_a = "a".repeat(80);
		let title = title_from_result(&sixty_a);
		assert_eq!(title.chars().count(), 60, "first line capped at 60 chars");
		assert!(title.starts_with('a'));
	}

	#[test]
	fn oversized_payloads_are_rejected_with_the_field_named() {
		assert!(ensure_within("note", "short", 16).is_ok());
		let err = ensure_within("note", &"x".repeat(17), 16).expect_err("over the limit");
		assert!(err.contains("note"), "{err}");
		assert!(err.contains("17 bytes"), "{err}");
	}
}
