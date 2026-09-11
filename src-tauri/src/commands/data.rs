use tauri::State;

use crate::db::{DEFAULT_LOG_QUESTIONS, SURVEY_QUESTIONS};
use crate::types::{
	DailyStatus, IdeaItem, LogAnswerItem, LogQuestionItem, TrialData, UserData,
};
use crate::AppState;

fn idea_list(ideas: Vec<IdeaItem>) -> serde_json::Value {
	serde_json::json!({ "ideas": ideas })
}

#[tauri::command]
pub fn get_user(state: State<'_, AppState>) -> UserData {
	let name = state.db.get_setting("user_name").filter(|s| !s.is_empty());
	let timezone = state.db.get_setting("user_timezone").filter(|s| !s.is_empty());
	let created_at = state
		.db
		.get_setting("created_at")
		.unwrap_or_else(|| "1970-01-01T00:00:00Z".into());
	UserData { email: None, name, mail_verified: true, timezone, created_at }
}

#[tauri::command]
pub fn get_user_trial() -> TrialData {
	TrialData { trial_end: None, is_paid: true }
}

#[tauri::command]
pub fn get_daily_status(state: State<'_, AppState>) -> DailyStatus {
	state.db.get_daily_status()
}

#[tauri::command]
pub fn get_daily_list(state: State<'_, AppState>) -> serde_json::Value {
	let items: Vec<serde_json::Value> = state
		.db
		.get_daily_list()
		.into_iter()
		.map(|(date, idea_id, log_id, created_at)| {
			serde_json::json!({
				"date": date,
				"idea_id": idea_id,
				"log_id": log_id,
				"created_at": created_at,
			})
		})
		.collect();
	serde_json::json!({ "week_start": "", "daily_intent": items })
}

#[tauri::command]
pub fn get_accountability() -> serde_json::Value {
	serde_json::json!({ "accountability_flag": false, "accountability_message": null })
}

#[tauri::command]
pub fn get_all_ideas(state: State<'_, AppState>) -> serde_json::Value {
	idea_list(state.db.list_ideas())
}

#[tauri::command]
pub fn get_idea(state: State<'_, AppState>, idea_id: String) -> Result<IdeaItem, String> {
	state
		.db
		.get_idea(&idea_id)
		.ok_or_else(|| format!("idea {idea_id} not found"))
}

#[tauri::command]
pub fn get_idea_children(state: State<'_, AppState>, idea_id: String) -> serde_json::Value {
	idea_list(state.db.get_idea_children(&idea_id))
}

#[tauri::command]
pub fn create_idea(
	state: State<'_, AppState>,
	result: Option<String>,
	transcript: Option<Vec<crate::types::ChatMessage>>,
	parent_idea_id: Option<String>,
	idea_metadata: Option<serde_json::Value>,
	idea_type: Option<String>,
	log_id: Option<String>,
) -> serde_json::Value {
	let id = uuid::Uuid::new_v4().to_string();
	let idea_type = idea_type.unwrap_or_else(|| "original".into());
	let result = result.unwrap_or_default();
	let transcript = transcript.unwrap_or_default();

	let mut title = String::new();
	if !result.is_empty() {
		title = title_from_result(&result);
	}
	state.db.insert_idea(
		&id,
		&title,
		&idea_type,
		&result,
		None,
		&transcript,
		&idea_metadata.unwrap_or_else(|| serde_json::json!({})),
		parent_idea_id.as_deref(),
		log_id.as_deref(),
		None,
		None,
		None,
	);

	if idea_type == "daily_intent" {
		state.db.set_daily_intent(&id);
		if !result.is_empty() {
			state.db.mark_daily_completed(&id);
		}
	}
	serde_json::json!({ "id": id })
}

#[tauri::command]
pub fn update_idea(
	state: State<'_, AppState>,
	id: String,
	title: Option<String>,
	result: Option<String>,
	transcript: Option<Vec<crate::types::ChatMessage>>,
	structured_result: Option<serde_json::Value>,
) -> serde_json::Value {
	let mut derived_title = title;
	if derived_title.is_none() {
		if let Some(r) = &result {
			if !r.is_empty() {
				let existing = state
					.db
					.get_idea(&id)
					.map(|i| i.title)
					.unwrap_or_default();				if existing.trim().is_empty() {
					derived_title = Some(title_from_result(r));
				}
			}
		}
	}
	state.db.update_idea(
		&id,
		derived_title.as_deref(),
		result.as_deref(),
		transcript.as_deref(),
		structured_result.as_ref(),
	);
	if let Some(r) = &result {
		if !r.is_empty() {
			state.db.mark_daily_completed(&id);
		}
	}
	serde_json::json!({ "id": id })
}

#[tauri::command]
pub fn mark_idea_read(state: State<'_, AppState>, idea_id: String) -> serde_json::Value {
	state.db.mark_idea_read(&idea_id);
	serde_json::json!({ "id": idea_id })
}

#[tauri::command]
pub fn delete_idea(state: State<'_, AppState>, idea_id: String) -> serde_json::Value {
	let deleted = state.db.delete_idea(&idea_id);
	serde_json::json!({ "id": idea_id, "deleted": deleted })
}

#[tauri::command]
pub fn get_log_questions(state: State<'_, AppState>) -> serde_json::Value {
	let enabled_ids = enabled_log_ids(&state);
	let answers = state.db.get_log_answers_today();
	let log: Vec<LogQuestionItem> = DEFAULT_LOG_QUESTIONS
		.iter()
		.map(|(id, text, label)| LogQuestionItem {
			id: *id,
			text: text.to_string(),
			label: label.to_string(),
			value: answers.as_ref().and_then(|a| {
				a.iter().find(|item| item.id == *id).map(|item| item.value)
			}),
		})
		.filter(|q| enabled_ids.contains(&q.id))
		.collect();
	serde_json::json!({ "log": log })
}

#[tauri::command]
pub fn submit_log(state: State<'_, AppState>, log: Vec<LogAnswerItem>) -> serde_json::Value {
	let id = uuid::Uuid::new_v4().to_string();
	state.db.insert_log(&id, &log);
	serde_json::json!({ "id": id })
}

#[tauri::command]
pub fn get_survey_fields() -> serde_json::Value {
	serde_json::json!({ "ids": SURVEY_QUESTIONS, "range": 5 })
}

#[tauri::command]
pub fn submit_survey(
	state: State<'_, AppState>,
	survey: serde_json::Value,
	idea_id: Option<String>,
) -> serde_json::Value {
	let id = uuid::Uuid::new_v4().to_string();
	state.db.insert_survey(&id, idea_id.as_deref(), &survey);
	serde_json::json!({ "id": id })
}

#[tauri::command]
pub fn get_notifications() -> serde_json::Value {
	// Sharing (and therefore share notifications) is removed in the desktop app.
	serde_json::json!({ "notifications": [] })
}

pub fn enabled_log_ids(state: &AppState) -> Vec<i64> {
	state
		.db
		.get_setting("enabled_log_question_ids")
		.and_then(|s| serde_json::from_str::<Vec<i64>>(&s).ok())
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
	result.lines().next().unwrap_or("").trim().chars().take(60).collect()
}
