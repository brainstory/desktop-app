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

/// One section reaction as it travels in a feedback share file (and as it
/// is handed to the database when such a file is imported).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SharedSectionReaction {
	/// index into the reacted idea's sections (`Db::result_to_json` order)
	pub section_index: i64,
	pub emoji: String,
}

/// A reaction on one section of an idea, as `get_reactions` returns it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionReaction {
	pub section_index: i64,
	pub emoji: String,
	/// the local user's own reaction (not carried in by a feedback file)
	pub mine: bool,
	/// creator name of the imported feedback that carried the reaction;
	/// None for the user's own reactions or when the name is unknown
	pub from: Option<String>,
}

/// The idea author's reaction on one comment (structured feedback item)
/// of a feedback child idea. Local only, never exported.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentReaction {
	pub feedback_idea_id: String,
	/// index into that feedback idea's `structured_result.feedback_items`
	pub item_index: i64,
	pub emoji: String,
}

/// Everything shown on one idea's page: reactions on its sections and on
/// the comments of its feedback children.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct IdeaReactions {
	pub sections: Vec<SectionReaction>,
	pub comments: Vec<CommentReaction>,
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

	#[test]
	fn get_reactions_shape_is_camel_case() {
		let value = serde_json::to_value(IdeaReactions {
			sections: vec![SectionReaction {
				section_index: 2,
				emoji: "👍".into(),
				mine: false,
				from: None,
			}],
			comments: vec![CommentReaction {
				feedback_idea_id: "f".into(),
				item_index: 0,
				emoji: "🚀".into(),
			}],
		})
		.unwrap();
		assert_eq!(
			value,
			serde_json::json!({
				"sections": [{ "sectionIndex": 2, "emoji": "👍", "mine": false, "from": null }],
				"comments": [{ "feedbackIdeaId": "f", "itemIndex": 0, "emoji": "🚀" }],
			})
		);
	}
}
