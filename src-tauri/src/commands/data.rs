use tauri::State;

use crate::db::{DEFAULT_LOG_QUESTIONS, SURVEY_QUESTIONS};
use crate::types::{DailyStatus, IdeaItem, IdeaType, LogAnswerItem, LogQuestionItem, UserData};
use crate::AppState;

fn idea_list(ideas: Vec<IdeaItem>) -> serde_json::Value {
	serde_json::json!({ "ideas": ideas })
}

#[tauri::command]
pub async fn get_user(state: State<'_, AppState>) -> Result<UserData, String> {
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
}

#[tauri::command]
pub async fn get_daily_status(state: State<'_, AppState>) -> Result<DailyStatus, String> {
	Ok(state.db.get_daily_status())
}

#[tauri::command]
pub async fn get_all_ideas(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
	Ok(idea_list(state.db.list_ideas()?))
}

#[tauri::command]
pub async fn get_idea(state: State<'_, AppState>, idea_id: String) -> Result<IdeaItem, String> {
	match state.db.get_idea(&idea_id)? {
		Some(idea) => Ok(idea),
		None => Err(format!("idea {idea_id} not found")),
	}
}

#[tauri::command]
pub async fn get_idea_children(
	state: State<'_, AppState>,
	idea_id: String,
) -> Result<serde_json::Value, String> {
	Ok(idea_list(state.db.get_idea_children(&idea_id)?))
}

#[tauri::command]
pub async fn create_idea(
	state: State<'_, AppState>,
	result: Option<String>,
	transcript: Option<Vec<crate::types::ChatMessage>>,
	parent_idea_id: Option<String>,
	idea_metadata: Option<serde_json::Value>,
	idea_type: Option<IdeaType>,
	log_id: Option<String>,
) -> Result<serde_json::Value, String> {
	let id = uuid::Uuid::new_v4().to_string();
	let idea_type = idea_type.unwrap_or_default();
	let result = result.unwrap_or_default();
	let transcript = transcript.unwrap_or_default();

	// (The parent's existence is verified inside insert_idea's transaction,
	// so a concurrent delete can't create an orphan.)

	let mut title = String::new();
	if !result.is_empty() {
		title = title_from_result(&result);
	}
	if idea_type == IdeaType::DailyIntent {
		state.db.create_daily_intent_idea(
			&id,
			&title,
			&result,
			&transcript,
			&idea_metadata.unwrap_or_else(|| serde_json::json!({})),
		)?;
	} else {
		state.db.insert_idea(crate::db::NewIdea {
			id: &id,
			title: &title,
			idea_type,
			result: &result,
			structured_result: None,
			transcript: &transcript,
			metadata: &idea_metadata.unwrap_or_else(|| serde_json::json!({})),
			parent_idea_id: parent_idea_id.as_deref(),
			log_id: log_id.as_deref(),
			..Default::default()
		})?;
	}

	Ok(serde_json::json!({ "id": id }))
}

#[tauri::command]
pub async fn update_idea(
	state: State<'_, AppState>,
	id: String,
	title: Option<String>,
	result: Option<String>,
	transcript: Option<Vec<crate::types::ChatMessage>>,
	structured_result: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
	// One lightweight read for the derived title, instead of parsing the
	// whole transcript; not-found is enforced inside the update
	// transaction (a concurrent delete can no longer slip past the check).
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
	state.db.update_idea(
		&id,
		derived_title.as_deref(),
		result.as_deref(),
		transcript.as_deref(),
		structured_result.as_ref(),
	)?;
	if let Some(r) = &result {
		if !r.is_empty() {
			state.db.mark_daily_completed(&id)?;
		}
	}
	Ok(serde_json::json!({ "id": id }))
}

#[tauri::command]
pub async fn mark_idea_read(
	state: State<'_, AppState>,
	idea_id: String,
) -> Result<serde_json::Value, String> {
	state.db.mark_idea_read(&idea_id)?;
	Ok(serde_json::json!({ "id": idea_id }))
}

#[tauri::command]
pub async fn delete_idea(
	state: State<'_, AppState>,
	idea_id: String,
) -> Result<serde_json::Value, String> {
	let deleted = state.db.delete_idea(&idea_id)?;
	Ok(serde_json::json!({ "id": idea_id, "deleted": deleted }))
}

#[tauri::command]
pub async fn get_log_questions(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
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
}

#[tauri::command]
pub async fn submit_log(
	state: State<'_, AppState>,
	log: Vec<LogAnswerItem>,
) -> Result<serde_json::Value, String> {
	let id = uuid::Uuid::new_v4().to_string();
	state.db.insert_log(&id, &log)?;
	Ok(serde_json::json!({ "id": id }))
}

/// Not called by the desktop UI (kept from the web-app API surface for
/// future parity; the reflection survey flow has no UI here yet).
#[tauri::command]
pub fn get_survey_fields() -> serde_json::Value {
	serde_json::json!({ "ids": SURVEY_QUESTIONS, "range": 5 })
}

#[tauri::command]
pub async fn submit_survey(
	state: State<'_, AppState>,
	survey: serde_json::Value,
	idea_id: Option<String>,
) -> Result<serde_json::Value, String> {
	let id = uuid::Uuid::new_v4().to_string();
	state.db.insert_survey(&id, idea_id.as_deref(), &survey)?;
	Ok(serde_json::json!({ "id": id }))
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
	use super::{enabled_log_ids, title_from_result};
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
}
