//! Emoji reactions people put on ideas: on an idea's sections (the
//! viewer's own, or carried in by an imported feedback file) and, by the
//! idea's author, on individual feedback comments. The LLM never picks
//! them; the set is fixed so every write and every import can be checked
//! against it.

/// Every reaction emoji the app accepts, with the meaning the UI shows.
/// The single source of truth: anything not listed here is rejected on
/// write and skipped on import. Note the warning sign carries the emoji
/// variation selector (U+FE0F); the bare U+26A0 is not accepted.
pub const REACTION_EMOJIS: [(&str, &str); 8] = [
	("\u{1F44D}", "agree"),
	("\u{1F44E}", "disagree"),
	("\u{2753}", "question"),
	("\u{1F4A1}", "suggestion"),
	("\u{1F615}", "confused"),
	("\u{26A0}\u{FE0F}", "error"),
	("\u{1F4DA}", "info"),
	("\u{1F680}", "action"),
];

/// True if `emoji` is exactly one of the accepted reaction strings.
pub fn is_valid_reaction(emoji: &str) -> bool {
	REACTION_EMOJIS.iter().any(|(e, _)| *e == emoji)
}

/// The error every write path returns for an emoji outside the set.
pub fn validate_reaction(emoji: &str) -> Result<(), String> {
	if is_valid_reaction(emoji) {
		Ok(())
	} else {
		Err(format!("unsupported reaction emoji: {emoji:?}"))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_set_is_exactly_the_eight_product_emojis() {
		let expected = [
			("👍", "agree"),
			("👎", "disagree"),
			("❓", "question"),
			("💡", "suggestion"),
			("😕", "confused"),
			("⚠️", "error"),
			("📚", "info"),
			("🚀", "action"),
		];
		assert_eq!(REACTION_EMOJIS, expected);
		for (emoji, _) in expected {
			assert!(is_valid_reaction(emoji), "{emoji} must be accepted");
			assert!(validate_reaction(emoji).is_ok());
		}
	}

	#[test]
	fn rejects_anything_outside_the_set() {
		// the warning sign without its variation selector is a different
		// string and must not slip through
		assert!(!is_valid_reaction("\u{26A0}"));
		for other in ["", " ", "👍 ", "👍👍", "agree", "❤️", "🙂", "<script>"] {
			assert!(!is_valid_reaction(other), "{other:?} must be rejected");
			assert!(validate_reaction(other).is_err());
		}
	}
}
