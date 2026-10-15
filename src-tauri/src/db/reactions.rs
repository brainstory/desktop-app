//! Stored emoji reactions on idea sections and feedback comments.

/// How many `#`/`##` sections the result document splits into - the
/// exclusive upper bound for a section index. Same heading rules as
/// the idea page's section splitting.
fn section_count(result: &str) -> i64 {
	let mut count = 0i64;
	let mut has_content = false;
	for line in result.lines() {
		let trimmed = line.trim_start();
		let is_heading =
			(trimmed.starts_with("# ") && !trimmed.starts_with("##")) || trimmed.starts_with("## ");
		if is_heading {
			count += 1;
			has_content = true;
		}
	}
	// a document with no headings at all is one section (its body)
	if has_content {
		count
	} else if !result.trim().is_empty() {
		1
	} else {
		0
	}
}

use rusqlite::{params, OptionalExtension};

use super::{now_iso, Db};
use crate::reactions::{
	validate_reaction, CommentReaction, IdeaReactions, SectionReaction, SharedSectionReaction,
};
use crate::types::IdeaType;

impl Db {
	/// Add the local user's `emoji` on section `section_index` of an idea,
	/// or remove it if already there. Returns true when it is now on.
	pub fn toggle_section_reaction(
		&self,
		idea_id: &str,
		section_index: i64,
		emoji: &str,
	) -> Result<bool, String> {
		validate_reaction(emoji)?;
		if section_index < 0 {
			return Err(format!("invalid section index {section_index}"));
		}
		let mut on = false;
		self.with_tx(|conn| {
			let result: Option<String> = conn
				.query_row(
					"SELECT result FROM ideas WHERE id = ?1",
					params![idea_id],
					|row| row.get(0),
				)
				.optional()
				.map_err(|e| format!("failed to read idea {idea_id}: {e}"))?;
			let Some(result) = result else {
				return Err(format!("idea {idea_id} not found"));
			};
			// index is bounded by the document's actual section count,
			// inside the transaction: a reaction can never land on a
			// section that does not exist
			let section_count = section_count(&result);
			if section_index >= section_count {
				return Err(format!(
					"invalid section index {section_index} (the document has {section_count} sections)"
				));
			}
			let removed = conn
				.execute(
					"DELETE FROM section_reactions
					 WHERE idea_id = ?1 AND section_index = ?2 AND emoji = ?3 AND source_idea_id IS NULL",
					params![idea_id, section_index, emoji],
				)
				.map_err(|e| format!("failed to save reaction: {e}"))?;
			if removed == 0 {
				conn.execute(
					"INSERT INTO section_reactions (idea_id, section_index, emoji, source_idea_id, created_at)
					 VALUES (?1, ?2, ?3, NULL, ?4)",
					params![idea_id, section_index, emoji, now_iso()],
				)
				.map_err(|e| format!("failed to save reaction: {e}"))?;
				on = true;
			}
			Ok(())
		})?;
		Ok(on)
	}

	/// Add the idea author's `emoji` on comment `item_index` of a feedback
	/// idea, or remove it if already there. Returns true when it is now on.
	pub fn toggle_comment_reaction(
		&self,
		feedback_idea_id: &str,
		item_index: i64,
		emoji: &str,
	) -> Result<bool, String> {
		validate_reaction(emoji)?;
		if item_index < 0 {
			return Err(format!("invalid comment index {item_index}"));
		}
		let mut on = false;
		self.with_tx(|conn| {
			let idea_type: Option<String> = conn
				.query_row(
					"SELECT idea_type FROM ideas WHERE id = ?1",
					params![feedback_idea_id],
					|row| row.get(0),
				)
				.optional()
				.map_err(|e| format!("failed to read idea {feedback_idea_id}: {e}"))?;
			match idea_type.as_deref() {
				None => return Err(format!("idea {feedback_idea_id} not found")),
				Some(t) if t != IdeaType::Feedback.as_str() => {
					return Err(format!(
						"idea {feedback_idea_id} is not feedback; only feedback comments take reactions"
					))
				}
				Some(_) => {}
			}
			let removed = conn
				.execute(
					"DELETE FROM comment_reactions
					 WHERE feedback_idea_id = ?1 AND item_index = ?2 AND emoji = ?3",
					params![feedback_idea_id, item_index, emoji],
				)
				.map_err(|e| format!("failed to save reaction: {e}"))?;
			if removed == 0 {
				conn.execute(
					"INSERT INTO comment_reactions (feedback_idea_id, item_index, emoji, created_at)
					 VALUES (?1, ?2, ?3, ?4)",
					params![feedback_idea_id, item_index, emoji, now_iso()],
				)
				.map_err(|e| format!("failed to save reaction: {e}"))?;
				on = true;
			}
			Ok(())
		})?;
		Ok(on)
	}

	/// Reactions shown on an idea's page: every reaction on its sections
	/// (the user's own and those carried in by imported feedback), plus
	/// the author's reactions on the comments of its feedback children.
	/// An unknown idea simply has none.
	pub fn get_reactions(&self, idea_id: &str) -> Result<IdeaReactions, String> {
		let conn = self.lock();
		let mut stmt = conn
			.prepare(
				"SELECT r.section_index, r.emoji, r.source_idea_id IS NULL, s.creator_name
				 FROM section_reactions r LEFT JOIN ideas s ON s.id = r.source_idea_id
				 WHERE r.idea_id = ?1
				 ORDER BY r.section_index, r.id",
			)
			.map_err(|e| format!("failed to read reactions: {e}"))?;
		let sections = stmt
			.query_map(params![idea_id], |row| {
				let mine: bool = row.get(2)?;
				Ok(SectionReaction {
					section_index: row.get(0)?,
					emoji: row.get(1)?,
					mine,
					from: if mine { None } else { row.get(3)? },
				})
			})
			.and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
			.map_err(|e| format!("failed to read reactions: {e}"))?;
		let mut stmt = conn
			.prepare(
				"SELECT c.feedback_idea_id, c.item_index, c.emoji
				 FROM comment_reactions c JOIN ideas f ON f.id = c.feedback_idea_id
				 WHERE f.parent_idea_id = ?1
				 ORDER BY c.feedback_idea_id, c.item_index, c.id",
			)
			.map_err(|e| format!("failed to read reactions: {e}"))?;
		let comments = stmt
			.query_map(params![idea_id], |row| {
				Ok(CommentReaction {
					feedback_idea_id: row.get(0)?,
					item_index: row.get(1)?,
					emoji: row.get(2)?,
				})
			})
			.and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
			.map_err(|e| format!("failed to read reactions: {e}"))?;
		Ok(IdeaReactions { sections, comments })
	}

	/// The local user's own section reactions on an idea (what a feedback
	/// export carries about its parent idea).
	pub fn my_section_reactions(
		&self,
		idea_id: &str,
	) -> Result<Vec<SharedSectionReaction>, String> {
		let conn = self.lock();
		let mut stmt = conn
			.prepare(
				"SELECT section_index, emoji FROM section_reactions
				 WHERE idea_id = ?1 AND source_idea_id IS NULL
				 ORDER BY section_index, id",
			)
			.map_err(|e| format!("failed to read reactions: {e}"))?;
		let rows = stmt
			.query_map(params![idea_id], |row| {
				Ok(SharedSectionReaction {
					section_index: row.get(0)?,
					emoji: row.get(1)?,
				})
			})
			.and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
			.map_err(|e| format!("failed to read reactions: {e}"))?;
		Ok(rows)
	}
}
