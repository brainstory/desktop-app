//! Single source of truth for every settings-table key used across the
//! backend. A typo in one of these strings silently forks a setting, so
//! they are named constants instead of literals scattered through modules.

pub mod setting {
	/// account creation timestamp (naive UTC, no trailing Z)
	pub const CREATED_AT: &str = "created_at";
	pub const USER_NAME: &str = "user_name";
	pub const USER_TIMEZONE: &str = "user_timezone";

	pub const REMINDER_ENABLED: &str = "reminder_enabled";
	pub const REMINDER_TIME: &str = "reminder_time";
	pub const REMINDER_LAST_FIRED: &str = "reminder_last_fired";

	pub const ENABLED_LOG_QUESTION_IDS: &str = "enabled_log_question_ids";

	pub const SHOW_IN_DOCK: &str = "show_in_dock";
	pub const SHOW_IN_TRAY: &str = "show_in_tray";

	/// automatic update checks ("false" = opted out; unset = on)
	pub const UPDATES_ENABLED: &str = "updates_enabled";

	pub const AI_LLM_MODE: &str = "ai_llm_mode";
	pub const AI_LLM_MODEL: &str = "ai_llm_model";
	pub const AI_STT_MODEL: &str = "ai_stt_model";
	pub const AI_STT_ENGINE: &str = "ai_stt_engine";
	pub const AI_STT_LANGUAGE: &str = "ai_stt_language";
	/// HuggingFace download endpoint override (mirror); empty = env/default
	pub const HF_ENDPOINT: &str = "hf_endpoint";
	pub const EXT_LLM_BASE_URL: &str = "ext_llm_base_url";
	pub const EXT_LLM_MODEL: &str = "ext_llm_model";
	pub const EXT_STT_BASE_URL: &str = "ext_stt_base_url";
	pub const EXT_STT_MODEL: &str = "ext_stt_model";

	/// legacy/plaintext fallback rows for the keychain-stored secrets
	pub mod secret {
		pub const HF_TOKEN: &str = "hf_token";
		pub const EXT_LLM_API_KEY: &str = "ext_llm_api_key";
		pub const EXT_STT_API_KEY: &str = "ext_stt_api_key";
	}
}

#[cfg(test)]
mod tests {
	use super::setting;

	#[test]
	fn all_keys_are_distinct() {
		let keys = [
			setting::CREATED_AT,
			setting::USER_NAME,
			setting::USER_TIMEZONE,
			setting::REMINDER_ENABLED,
			setting::REMINDER_TIME,
			setting::REMINDER_LAST_FIRED,
			setting::ENABLED_LOG_QUESTION_IDS,
			setting::SHOW_IN_DOCK,
			setting::SHOW_IN_TRAY,
			setting::UPDATES_ENABLED,
			setting::AI_LLM_MODE,
			setting::AI_LLM_MODEL,
			setting::AI_STT_MODEL,
			setting::AI_STT_ENGINE,
			setting::AI_STT_LANGUAGE,
			setting::HF_ENDPOINT,
			setting::EXT_LLM_BASE_URL,
			setting::EXT_LLM_MODEL,
			setting::EXT_STT_BASE_URL,
			setting::EXT_STT_MODEL,
			setting::secret::HF_TOKEN,
			setting::secret::EXT_LLM_API_KEY,
			setting::secret::EXT_STT_API_KEY,
		];
		let mut sorted = keys.to_vec();
		sorted.sort_unstable();
		let before = sorted.len();
		sorted.dedup();
		assert_eq!(sorted.len(), before, "settings keys must be unique");
	}
}
