/// A single message in a conversation transcript.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ChatMessage {
	pub role: String,
	pub content: String,
}

/// What kind of idea a row is. Stored in `ideas.idea_type` and sent over
/// IPC / in share files as the snake_case strings "original",
/// "feedback" and "daily_intent".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeaType {
	/// a brainstorm of the user's own (also every imported idea)
	#[default]
	Original,
	/// a reaction to another idea (`parent_idea_id` is set)
	Feedback,
	/// the day's intention
	DailyIntent,
}

impl IdeaType {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Original => "original",
			Self::Feedback => "feedback",
			Self::DailyIntent => "daily_intent",
		}
	}
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
	pub updates: UpdatesSettings,
}

/// Automatic update checks (the frontend updater banner reads this).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatesSettings {
	pub enabled: bool,
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

#[cfg(test)]
mod tests {
	use super::IdeaType;

	#[test]
	fn idea_types_travel_as_the_stored_strings() {
		for (ty, raw) in [
			(IdeaType::Original, "original"),
			(IdeaType::Feedback, "feedback"),
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
