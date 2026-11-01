/// The user message recorded when someone asks for a different question
/// (mirrors ASK_A_DIFFERENT_QUESTION in src/const.ts; a test keeps the
/// two in sync): meta, not an answer, so titles and exports skip it.
pub const ASK_A_DIFFERENT_QUESTION: &str = "Ask me a different question!";

/// A single message in a conversation transcript.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct ChatMessage {
	pub role: String,
	pub content: String,
}

/// What kind of idea a row is. Stored in `ideas.idea_type` and sent over
/// IPC / in share files as the snake_case strings "original",
/// "feedback", "continuation" and "daily_intent".
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub enum IdeaType {
	/// a brainstorm of the user's own (also every imported idea)
	#[default]
	Original,
	/// a reaction to another idea (`parent_idea_id` is set)
	Feedback,
	/// a new session continuing an existing idea
	Continuation,
	/// the day's intention
	DailyIntent,
}

impl IdeaType {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Original => "original",
			Self::Feedback => "feedback",
			Self::Continuation => "continuation",
			Self::DailyIntent => "daily_intent",
		}
	}
}

/// A note the user attached to an idea. Stored structurally in the
/// idea's metadata (`notes`), and rendered into the writeup's trailing
/// "## Notes" section on every change - so the Summary tab and exports
/// keep carrying them, while the note list stays editable.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct IdeaNote {
	pub id: String,
	pub text: String,
	pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct IdeaItem {
	pub id: String,
	pub title: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub result: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub r#type: Option<String>,
	pub created_at: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub creator_email: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub creator_name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub is_unread: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub transcript: Option<Vec<ChatMessage>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub shared_with_users: Option<Vec<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(type = "unknown", optional)]
	pub structured_result: Option<serde_json::Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(type = "unknown", optional)]
	pub result_json: Option<serde_json::Value>,
	/// the user's structured notes (see [`IdeaNote`]); absent when there
	/// are none
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub notes: Option<Vec<IdeaNote>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub parent_idea: Option<Box<IdeaItem>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub feedback: Option<Vec<IdeaItem>>,
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct UserData {
	pub name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub timezone: Option<String>,
	pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct DailyStatus {
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub log_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub intent_idea_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub survey_id: Option<String>,
	pub is_completed: bool,
	#[ts(type = "number")]
	pub streak: i64,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct LogAnswerItem {
	#[ts(type = "number")]
	pub id: i64,
	pub value: bool,
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct LogQuestionItem {
	#[ts(type = "number")]
	pub id: i64,
	pub text: String,
	pub label: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub value: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct LogSettingsItem {
	#[ts(type = "number")]
	pub id: i64,
	pub label: String,
	pub text: String,
	pub enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct NotificationSettingsItem {
	pub title: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub description: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub value: Option<String>,
	pub value_type: String,
	pub enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct UserSettings {
	pub user: UserSettingsUser,
	pub log: Vec<LogSettingsItem>,
	pub notifications: Vec<NotificationSettingsItem>,
	pub presence: AppPresence,
	pub updates: UpdatesSettings,
	/// the global quick capture shortcut is enabled
	pub quick_capture: bool,
}

/// Automatic update checks (the frontend updater banner reads this).
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct UpdatesSettings {
	pub enabled: bool,
}

/// Where the app shows up on the desktop (macOS).
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct AppPresence {
	pub dock: bool,
	pub tray: bool,
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct UserSettingsUser {
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub timezone: Option<String>,
}

/// Streaming event sent to the webview. Mirrors the websocket packet
/// format of the original brainstory backend: {type, content, timestamp}.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct StreamEvent {
	pub r#type: String,
	pub content: String,
	pub timestamp: String,
}

impl StreamEvent {
	pub fn new(event_type: &str, content: impl Into<String>) -> Self {
		Self {
			r#type: event_type.to_string(),
			content: content.into(),
			timestamp: chrono::Utc::now().to_rfc3339(),
		}
	}
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub struct ModelStatus {
	pub id: String,
	pub label: String,
	pub description: String,
	pub kind: String,
	#[ts(type = "number")]
	pub size_bytes: u64,
	pub downloaded: bool,
	pub active: bool,
	/// true while a download is in flight
	pub downloading: bool,
	/// a user pause stopped the transfer; `progress` carries where it
	/// stopped, and resuming continues from the kept `.part`
	pub paused: bool,
	/// download progress 0-100 (the live pct while downloading, the
	/// stop point while paused)
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub progress: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional)]
	pub filename: Option<String>,
	/// Approximate KV-cache bytes per token for local LLM rows (absent
	/// for STT): window tokens x this = the extra memory a context
	/// window needs.
	#[serde(skip_serializing_if = "Option::is_none")]
	#[ts(optional, type = "number")]
	pub kv_bytes_per_token: Option<f64>,
}

#[cfg(test)]
mod tests {
	use super::{IdeaType, ASK_A_DIFFERENT_QUESTION};

	#[test]
	fn skip_message_matches_the_frontend_constant() {
		let frontend = include_str!("../../src/const.ts");
		assert!(
			frontend.contains(&format!(
				"export const ASK_A_DIFFERENT_QUESTION = \"{ASK_A_DIFFERENT_QUESTION}\";"
			)),
			"src/const.ts and types.rs disagree on the skip message"
		);
	}

	#[test]
	fn idea_types_travel_as_the_stored_strings() {
		for (ty, raw) in [
			(IdeaType::Original, "original"),
			(IdeaType::Feedback, "feedback"),
			(IdeaType::Continuation, "continuation"),
			(IdeaType::DailyIntent, "daily_intent"),
		] {
			assert_eq!(ty.as_str(), raw);
			assert_eq!(serde_json::to_value(ty).unwrap(), raw);
			assert_eq!(serde_json::from_value::<IdeaType>(raw.into()).unwrap(), ty);
		}
		assert!(serde_json::from_value::<IdeaType>("dailyintent".into()).is_err());
		assert_eq!(IdeaType::default(), IdeaType::Original);
	}
}
