/// A single message in a conversation transcript.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ChatMessage {
	pub role: String,
	pub content: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct IdeaItem {
	pub id: String,
	pub title: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub result: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub r#type: Option<String>,
	pub created_at: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub creator_email: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub creator_name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub is_unread: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub transcript: Option<Vec<ChatMessage>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub shared_with_users: Option<Vec<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub structured_result: Option<serde_json::Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub result_json: Option<serde_json::Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_idea: Option<Box<IdeaItem>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub feedback: Option<Vec<IdeaItem>>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct UserData {
	pub email: Option<String>,
	pub name: Option<String>,
	pub mail_verified: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub timezone: Option<String>,
	pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct DailyStatus {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub log_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub intent_idea_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub survey_id: Option<String>,
	pub is_completed: bool,
	pub streak: i64,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct LogAnswerItem {
	pub id: i64,
	pub value: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LogQuestionItem {
	pub id: i64,
	pub text: String,
	pub label: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub value: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LogSettingsItem {
	pub id: i64,
	pub label: String,
	pub text: String,
	pub enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct NotificationSettingsItem {
	pub title: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub value: Option<String>,
	pub value_type: String,
	pub enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSettings {
	pub user: UserSettingsUser,
	pub log: Vec<LogSettingsItem>,
	pub notifications: Vec<NotificationSettingsItem>,
	pub presence: AppPresence,
}

/// Where the app shows up on the desktop (macOS).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPresence {
	pub dock: bool,
	pub tray: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct UserSettingsUser {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub timezone: Option<String>,
}

/// Streaming event sent to the webview. Mirrors the websocket packet
/// format of the original brainstory backend: {type, content, timestamp}.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
	pub id: String,
	pub label: String,
	pub description: String,
	pub kind: String,
	pub size_bytes: u64,
	pub downloaded: bool,
	pub active: bool,
	/// true while a download is in flight
	pub downloading: bool,
	/// download progress 0-100, only meaningful while downloading
	#[serde(skip_serializing_if = "Option::is_none")]
	pub progress: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub filename: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
pub struct RuntimeStatus {
	pub state: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
}
