use crate::types::ChatMessage;

pub const STORY_INTERVIEW_SYSTEM: &str =
	include_str!("../../prompts/story_interview_system_message.txt");
pub const STORY_INTERVIEW_CONTEXT_SYSTEM: &str =
	include_str!("../../prompts/story_interview_context_system_message.txt");
pub const STORY_INTERVIEW_REACT_SYSTEM: &str =
	include_str!("../../prompts/story_interview_react_system_message.txt");
pub const STORY_RESULT_SYSTEM: &str = include_str!("../../prompts/story_result_system_message.txt");
pub const FEEDBACK_RESULT_SYSTEM: &str =
	include_str!("../../prompts/feedback_result_system_message.txt");
pub const FEEDBACK_JSON_RESULT_SYSTEM: &str =
	include_str!("../../prompts/feedback_json_result_system_message.txt");

/// Chat type communicated by the frontend (mirrors CHAT_TYPE in src/const.js).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatType {
	Original,
	Feedback,
	DailyIntent,
}

impl ChatType {
	pub fn parse(value: Option<&str>) -> Self {
		match value {
			Some("daily_intent") => ChatType::DailyIntent,
			Some("feedback") => ChatType::Feedback,
			_ => ChatType::Original,
		}
	}
}

/// User content interpolated into the tag-structured prompts can't be
/// allowed to break the tag structure (e.g. an imported idea containing
/// `</idea>` or an opening `<oid ...>`), so both closing and opening
/// markers are neutralized.
fn sanitize_tag_content(content: &str, tag: &str) -> String {
	let close = format!("</{tag}>");
	let open_exact = format!("<{tag}>");
	// also catches attribute forms like <idea author="...">
	let open_attr = format!("<{tag} ");
	content
		.replace(&close, &format!("<\\{tag}>"))
		.replace(&open_exact, &format!("<\\{tag}>"))
		.replace(&open_attr, &format!("<\\{tag} "))
}

/// Options describing one LLM call, resolved by the ai command layer.
#[derive(Debug, Clone)]
pub struct PromptRequest {
	pub chat_type: ChatType,
	pub messages: Vec<ChatMessage>,
	pub summarize: bool,
	pub react_to: Option<String>,
	pub react_to_author: Option<String>,
	pub react_to_is_current_user: bool,
	pub structured_feedback: bool,
}

impl PromptRequest {
	/// Build the system prompt for this request following the original
	/// brainstory prompting system.
	pub fn system_prompt(&self) -> String {
		if self.summarize {
			if self.chat_type == ChatType::Feedback {
				if self.structured_feedback {
					return FEEDBACK_JSON_RESULT_SYSTEM.to_string();
				}
				return FEEDBACK_RESULT_SYSTEM.to_string();
			}
			return STORY_RESULT_SYSTEM.to_string();
		}

		match self.chat_type {
			ChatType::Original => STORY_INTERVIEW_SYSTEM.to_string(),
			ChatType::DailyIntent => STORY_INTERVIEW_CONTEXT_SYSTEM.to_string(),
			ChatType::Feedback => {
				let author = self
					.react_to_author
					.clone()
					.unwrap_or_else(|| "the author".into());
				let is_current_user = if self.react_to_is_current_user {
					"true"
				} else {
					"false"
				};
				let idea = sanitize_tag_content(&self.react_to.clone().unwrap_or_default(), "idea");
				format!(
					"{}\n\n<idea author=\"{}\" is_current_user=\"{}\">{}</idea>",
					STORY_INTERVIEW_REACT_SYSTEM.trim_end(),
					author.replace('"', "'"),
					is_current_user,
					idea
				)
			}
		}
	}

	/// Build the user message list sent to the model.
	pub fn user_messages(&self) -> Vec<ChatMessage> {
		if self.summarize {
			let transcript = serde_json::to_string(&self.messages).unwrap_or_else(|_| "[]".into());
			let mut content = format!("<t>{}</t>", sanitize_tag_content(&transcript, "t"));
			if self.chat_type == ChatType::Feedback {
				if let Some(oid) = &self.react_to {
					let author = self
						.react_to_author
						.clone()
						.unwrap_or_else(|| "the author".into());
					let is_current_user = if self.react_to_is_current_user {
						"true"
					} else {
						"false"
					};
					content = format!(
						"<oid oida=\"{}\" is_current_user=\"{}\">{}</oid>\n{}",
						author.replace('"', "'"),
						is_current_user,
						sanitize_tag_content(oid, "oid"),
						content
					);
				}
			}
			return vec![ChatMessage {
				role: "user".into(),
				content,
			}];
		}
		self.messages.clone()
	}
}

#[cfg(test)]
mod tests {
	use super::sanitize_tag_content;

	#[test]
	fn neutralizes_closing_tags() {
		assert_eq!(sanitize_tag_content("a </idea> b", "idea"), "a <\\idea> b");
	}

	#[test]
	fn neutralizes_opening_tags() {
		assert_eq!(
			sanitize_tag_content("x <oid oida=\"evil\"> y", "oid"),
			"x <\\oid oida=\"evil\"> y"
		);
		assert_eq!(sanitize_tag_content("x <t> y", "t"), "x <\\t> y");
	}

	#[test]
	fn leaves_unrelated_angle_brackets_alone() {
		assert_eq!(
			sanitize_tag_content("use <b>bold</b>", "idea"),
			"use <b>bold</b>"
		);
		assert_eq!(sanitize_tag_content("math: 5 < 10", "t"), "math: 5 < 10");
	}
}
