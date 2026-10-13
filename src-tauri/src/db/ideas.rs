//! Idea storage: creation, drafts, updates, notes, deletion, library
//! listing and the feedback tree.

use rusqlite::{params, Connection, OptionalExtension};

use super::{local_date_for, now_iso, Db, NewIdea};
use crate::reactions::validate_reaction;
use crate::types::{ChatMessage, IdeaItem, IdeaType};
use chrono_tz::Tz;

impl Db {
	/// Split a markdown result into {heading, body} sections for the idea
	/// page's document view (index 0 is the document title, so the feedback
	/// JSON's 1-based heading ordinals line up with array indices).
	pub(super) fn result_to_json(result: &str) -> serde_json::Value {
		let mut items: Vec<(String, String)> = Vec::new();
		let mut current_heading = String::new();
		let mut current_body = String::new();

		for line in result.lines() {
			let trimmed = line.trim_start();
			let is_heading = (trimmed.starts_with("# ") && !trimmed.starts_with("##"))
				|| trimmed.starts_with("## ");
			if is_heading {
				if !current_heading.is_empty() || !current_body.trim().is_empty() {
					items.push((current_heading.clone(), current_body.trim().to_string()));
				}
				current_heading = trimmed.to_string();
				current_body.clear();
			} else {
				current_body.push_str(line);
				current_body.push('\n');
			}
		}
		if !current_heading.is_empty() || !current_body.trim().is_empty() {
			items.push((current_heading, current_body.trim().to_string()));
		}
		// A result that starts directly with a heading would otherwise shift
		// every section by one; keep index 0 as the (empty) title slot.
		if let Some((heading, _)) = items.first() {
			if heading.starts_with('#') {
				items.insert(0, (String::new(), String::new()));
			}
		}

		serde_json::to_value(
			items
				.into_iter()
				.map(|(heading, body)| serde_json::json!({ "heading": heading, "body": body }))
				.collect::<Vec<_>>(),
		)
		.unwrap_or_else(|_| serde_json::json!([]))
	}

	fn row_to_idea(row: &rusqlite::Row) -> rusqlite::Result<(Option<String>, IdeaItem)> {
		let transcript: String = row.get("transcript")?;
		let structured: Option<String> = row.get("structured_result")?;
		let result: String = row.get("result")?;
		let result_json = if result.is_empty() {
			None
		} else {
			Some(Self::result_to_json(&result))
		};
		Ok((
			row.get("parent_idea_id")?,
			IdeaItem {
				id: row.get("id")?,
				title: row.get("title")?,
				result: Some(result),
				r#type: Some(row.get::<_, String>("idea_type")?),
				created_at: row.get("created_at")?,
				creator_email: row.get("creator_email")?,
				creator_name: row.get("creator_name")?,
				is_unread: Some(row.get::<_, i64>("is_unread")? != 0),
				transcript: serde_json::from_str::<Vec<ChatMessage>>(&transcript).ok(),
				shared_with_users: None,
				structured_result: structured.and_then(|s| serde_json::from_str(&s).ok()),
				result_json,
				parent_idea: None,
				feedback: None,
			},
		))
	}

	/// Library-list rows: the transcript only for drafts (the grid previews
	/// a draft's last user message), and no structured or sectioned
	/// result - nothing in the list reads them, and decoding every
	/// transcript and result document made each dashboard load scale with
	/// the whole library's text.
	fn row_to_summary(row: &rusqlite::Row) -> rusqlite::Result<(Option<String>, IdeaItem)> {
		let transcript: Option<String> = row.get("transcript")?;
		Ok((
			row.get("parent_idea_id")?,
			IdeaItem {
				id: row.get("id")?,
				title: row.get("title")?,
				result: Some(row.get("result")?),
				r#type: Some(row.get::<_, String>("idea_type")?),
				created_at: row.get("created_at")?,
				creator_email: row.get("creator_email")?,
				creator_name: row.get("creator_name")?,
				is_unread: Some(row.get::<_, i64>("is_unread")? != 0),
				transcript: transcript
					.and_then(|t| serde_json::from_str::<Vec<ChatMessage>>(&t).ok()),
				shared_with_users: None,
				structured_result: None,
				result_json: None,
				parent_idea: None,
				feedback: None,
			},
		))
	}

	/// Library-list rows: a bounded result preview (the grid shows a
	/// snippet, never the whole writeup - a large library otherwise
	/// ships every result through JSON on each dashboard load), the
	/// transcript only for drafts (the grid previews a draft's last user
	/// message), and no structured or sectioned result - nothing in the
	/// list reads them.
	const IDEA_SUMMARY_COLS: &'static str = "id, title, idea_type, substr(result, 1, 600) AS result, CASE WHEN result = '' THEN transcript END AS transcript, parent_idea_id, is_unread, creator_name, creator_email, created_at";

	const IDEA_COLS: &'static str =
		"id, title, idea_type, result, structured_result, transcript, idea_metadata, parent_idea_id, log_id, is_unread, creator_name, creator_email, share_id, created_at";

	fn insert_idea_tx(
		conn: &Connection,
		idea: NewIdea<'_>,
		zone: Option<Tz>,
	) -> Result<(), String> {
		let transcript_json =
			serde_json::to_string(idea.transcript).unwrap_or_else(|_| "[]".into());
		let structured_json = idea.structured_result.map(|v| v.to_string());
		let now = now_iso();
		let created_at = idea.created_at.unwrap_or(&now);
		// Imported rows keep their original timestamp but never count as
		// the importer's own activity: an empty local_date excludes them
		// from the streak and has_activity_today queries.
		let local_date = if idea.imported {
			String::new()
		} else {
			local_date_for(created_at, zone)
		};
		conn.execute(
			"INSERT INTO ideas (id, title, idea_type, result, structured_result, transcript, idea_metadata, parent_idea_id, log_id, is_unread, creator_name, creator_email, share_id, created_at, local_date)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
			params![
				idea.id,
				idea.title,
				idea.idea_type.as_str(),
				idea.result,
				structured_json,
				transcript_json,
				idea.metadata.to_string(),
				idea.parent_idea_id,
				idea.log_id,
				idea.unread as i64,
				idea.creator_name,
				idea.creator_email,
				idea.share_id,
				created_at,
				local_date,
			],
		)
		.map_err(|e| format!("failed to save idea: {e}"))?;
		if !idea.parent_section_reactions.is_empty() {
			let parent_id = idea
				.parent_idea_id
				.ok_or("section reactions need a parent idea to attach to")?;
			for reaction in idea.parent_section_reactions {
				validate_reaction(&reaction.emoji)?;
				if reaction.section_index < 0 {
					return Err(format!("invalid section index {}", reaction.section_index));
				}
				// OR IGNORE: a repeated entry is the same reaction
				conn.execute(
					"INSERT OR IGNORE INTO section_reactions (idea_id, section_index, emoji, source_idea_id, created_at)
					 VALUES (?1, ?2, ?3, ?4, ?5)",
					params![
						parent_id,
						reaction.section_index,
						reaction.emoji,
						idea.id,
						now
					],
				)
				.map_err(|e| format!("failed to save reactions: {e}"))?;
			}
		}
		Ok(())
	}

	/// Insert an idea. A `parent_idea_id` is verified inside the same
	/// transaction, so a concurrent delete can't slip an orphan through.
	/// `NewIdea::imported()` builds the share-file variant, whose row
	/// never counts toward the importer's own activity (its local_date
	/// stays empty, excluding it from streak/has_activity queries)
	/// because the activity happened on the author's machine.
	pub fn insert_idea(&self, idea: NewIdea<'_>) -> Result<(), String> {
		self.with_tx(|conn| {
			if let Some(parent_id) = idea.parent_idea_id {
				let exists: i64 = conn
					.query_row(
						"SELECT EXISTS(SELECT 1 FROM ideas WHERE id = ?1)",
						params![parent_id],
						|row| row.get(0),
					)
					.map_err(|e| format!("failed to verify parent idea: {e}"))?;
				if exists == 0 {
					return Err(format!("parent idea {parent_id} not found"));
				}
			}
			Self::insert_idea_tx(conn, idea, self.active_zone())
		})
	}

	/// Insert an idea that is today's daily intent (single transaction:
	/// idea + daily row stay consistent).
	#[allow(clippy::too_many_arguments)]
	pub fn create_daily_intent_idea(
		&self,
		id: &str,
		title: &str,
		result: &str,
		transcript: &[ChatMessage],
		metadata: &serde_json::Value,
	) -> Result<(), String> {
		let today = self.today_local().format("%Y-%m-%d").to_string();
		let mark_completed = !result.is_empty();
		let zone = self.active_zone();
		self.with_tx(|conn| {
			Self::insert_idea_tx(
				conn,
				NewIdea {
					id,
					title,
					idea_type: IdeaType::DailyIntent,
					result,
					transcript,
					metadata,
					..Default::default()
				},
				zone,
			)?;
			conn.execute(
				// A new intent for the day replaces the old one as a fresh
				// draft: is_completed must reset, or a later empty draft
				// still shows the day as finished.
				"INSERT INTO daily (date, intent_idea_id, is_completed, created_at) VALUES (?1, ?2, 0, ?3)
				 ON CONFLICT(date) DO UPDATE SET intent_idea_id = ?2, is_completed = 0",
				params![today, id, now_iso()],
			)
			.map_err(|e| format!("failed to save daily intent: {e}"))?;
			if mark_completed {
				conn.execute(
					"UPDATE daily SET is_completed = 1 WHERE date = ?1 AND intent_idea_id = ?2",
					params![today, id],
				)
				.map_err(|e| format!("failed to complete daily intent: {e}"))?;
			}
			Ok(())
		})
	}

	/// Update an idea's fields in one transaction. Saving a nonempty
	/// result is the finish gesture: when this idea is today's daily
	/// intent, the day is completed inside the same transaction, so the
	/// idea and the day's completion state commit or roll back together.
	pub fn update_idea(
		&self,
		id: &str,
		title: Option<&str>,
		result: Option<&str>,
		transcript: Option<&[ChatMessage]>,
		structured_result: Option<&serde_json::Value>,
	) -> Result<(), String> {
		self.with_tx(|conn| {
			// Existence enforced inside the transaction: the check and the
			// update can no longer be split by a concurrent delete.
			let exists: i64 = conn
				.query_row(
					"SELECT EXISTS(SELECT 1 FROM ideas WHERE id = ?1)",
					params![id],
					|row| row.get(0),
				)
				.map_err(|e| format!("failed to read idea {id}: {e}"))?;
			if exists == 0 {
				return Err(format!("idea {id} not found"));
			}
			if let Some(t) = title {
				conn.execute("UPDATE ideas SET title = ?1 WHERE id = ?2", params![t, id])
					.map_err(|e| format!("failed to save title: {e}"))?;
			}
			if let Some(r) = result {
				conn.execute("UPDATE ideas SET result = ?1 WHERE id = ?2", params![r, id])
					.map_err(|e| format!("failed to save result: {e}"))?;
			}
			if let Some(t) = transcript {
				let json = serde_json::to_string(t).unwrap_or_else(|_| "[]".into());
				conn.execute(
					"UPDATE ideas SET transcript = ?1 WHERE id = ?2",
					params![json, id],
				)
				.map_err(|e| format!("failed to save transcript: {e}"))?;
			}
			if let Some(s) = structured_result {
				conn.execute(
					"UPDATE ideas SET structured_result = ?1 WHERE id = ?2",
					params![s.to_string(), id],
				)
				.map_err(|e| format!("failed to save feedback document: {e}"))?;
			}
			// Same predicate the command always applied: only a nonempty
			// result marks the day complete. An idea with no (or another
			// day's) daily association matches zero rows and is a no-op.
			if let Some(r) = result {
				if !r.is_empty() {
					let today = self.today_local();
					Self::complete_daily_tx(conn, id, today)?;
				}
			}
			Ok(())
		})
	}

	pub fn mark_idea_read(&self, id: &str) -> Result<(), String> {
		self.lock()
			.execute("UPDATE ideas SET is_unread = 0 WHERE id = ?1", params![id])
			.map_err(|e| format!("failed to mark idea read: {e}"))?;
		Ok(())
	}

	/// Append a user note to an idea's writeup as a "## Notes" section
	/// (created on first use), reading and writing under one transaction
	/// so a concurrent save can't overwrite the note with a stale result.
	/// Notes live in the same markdown result as the writeup: they show
	/// in the idea's Summary tab and travel with exports, exactly like a
	/// section the model wrote.
	pub fn append_idea_note(&self, id: &str, note: &str) -> Result<(), String> {
		self.with_tx(|conn| {
			let result: String = conn
				.query_row(
					"SELECT result FROM ideas WHERE id = ?1",
					params![id],
					|row| row.get(0),
				)
				.optional()
				.map_err(|e| format!("failed to read idea {id}: {e}"))?
				.ok_or_else(|| format!("idea {id} not found"))?;
			let updated = Self::append_note_to_result(&result, note);
			conn.execute(
				"UPDATE ideas SET result = ?1 WHERE id = ?2",
				params![updated, id],
			)
			.map_err(|e| format!("failed to save the note: {e}"))?;
			Ok(())
		})
	}

	/// `result` with `note` appended to its trailing "## Notes" section,
	/// creating the heading when absent. Appending at the end of the
	/// document is what keeps every note inside the one section: notes
	/// are only ever added there.
	fn append_note_to_result(result: &str, note: &str) -> String {
		let base = result.trim_end();
		if base.is_empty() {
			return format!("## Notes\n\n{note}");
		}
		if base.lines().any(|line| line.trim() == "## Notes") {
			return format!("{base}\n\n{note}");
		}
		format!("{base}\n\n## Notes\n\n{note}")
	}

	/// Delete an idea and its whole feedback subtree in one transaction
	/// (recursive, so a child-of-child can never survive as an orphan).
	/// `surveys.idea_id` and `daily.intent_idea_id` are nullable idea
	/// references with no enforcing foreign key, so both are cleared for
	/// every subtree member - clearing only the requested id would leave
	/// links to a deleted descendant dangling. The subtree (the
	/// requested id plus every descendant) is computed once, before any
	/// row is removed, and every statement below applies to that same
	/// set; reactions need no handling here because their foreign keys
	/// cascade.
	pub fn delete_idea(&self, id: &str) -> Result<bool, String> {
		let mut deleted = false;
		self.with_tx(|conn| {
			let subtree: Vec<String> = {
				let mut stmt = conn
					.prepare(
						"WITH RECURSIVE subtree(id) AS (
							SELECT id FROM ideas WHERE id = ?1
							UNION ALL
							SELECT i.id FROM ideas i JOIN subtree s ON i.parent_idea_id = s.id
						)
						SELECT id FROM subtree",
					)
					.map_err(|e| format!("failed to delete idea: {e}"))?;
				let ids: Vec<String> = stmt
					.query_map(params![id], |row| row.get(0))
					.map_err(|e| format!("failed to delete idea: {e}"))?
					.collect::<Result<_, _>>()
					.map_err(|e| format!("failed to delete idea: {e}"))?;
				ids
			};
			// a missing id deletes nothing (but still cleans up any
			// orphans a pre-foreign-key database left under it)
			deleted = subtree.iter().any(|sid| sid.as_str() == id);
			// json_each binds the whole set as one parameter; splicing
			// ids into the SQL text would let an id containing SQL
			// syntax change the statement
			let ids = serde_json::to_string(&subtree)
				.map_err(|e| format!("failed to delete idea: {e}"))?;
			for sql in [
				"UPDATE daily SET intent_idea_id = NULL
				 WHERE intent_idea_id IN (SELECT value FROM json_each(?1))",
				"UPDATE surveys SET idea_id = NULL
				 WHERE idea_id IN (SELECT value FROM json_each(?1))",
				"DELETE FROM ideas
				 WHERE id IN (SELECT value FROM json_each(?1))",
			] {
				conn.execute(sql, params![ids])
					.map_err(|e| format!("failed to delete idea: {e}"))?;
			}
			Ok(())
		})?;
		Ok(deleted)
	}

	/// Fetch one idea. `Ok(None)` means genuinely not found; a SQL/decode
	/// failure is an Err so the UI can distinguish "deleted" from "broken".
	pub fn get_idea(&self, id: &str) -> Result<Option<IdeaItem>, String> {
		let conn = self.lock();
		let row = conn
			.query_row(
				&format!("SELECT {} FROM ideas WHERE id = ?1", Self::IDEA_COLS),
				params![id],
				Self::row_to_idea,
			)
			.optional()
			.map_err(|e| format!("failed to read idea {id}: {e}"))?;
		let Some((parent_id, mut idea)) = row else {
			return Ok(None);
		};

		if let Some(parent_id) = parent_id {
			let parent = conn
				.query_row(
					&format!("SELECT {} FROM ideas WHERE id = ?1", Self::IDEA_COLS),
					params![parent_id],
					Self::row_to_idea,
				)
				.optional()
				.map_err(|e| format!("failed to read parent idea {parent_id}: {e}"))?;
			if let Some((_, parent)) = parent {
				idea.parent_idea = Some(Box::new(parent));
			}
		}
		Ok(Some(idea))
	}

	/// Find an idea by its share id (used when importing feedback that
	/// references an exported idea). `Ok(None)` means genuinely absent; a
	/// read failure is an Err so "not found" and "broken" stay distinct.
	pub fn get_idea_by_share_id(&self, share_id: &str) -> Result<Option<IdeaItem>, String> {
		let conn = self.lock();
		conn.query_row(
			&format!(
				"SELECT {} FROM ideas WHERE share_id = ?1 OR id = ?1 ORDER BY share_id IS NULL",
				Self::IDEA_COLS
			),
			params![share_id],
			Self::row_to_idea,
		)
		.optional()
		.map_err(|e| format!("failed to look up the shared idea: {e}"))
		.map(|row| row.map(|(_, idea)| idea))
	}

	/// Root ideas whose title matches exactly, for the feedback import's
	/// unambiguous title fallback. `Ok(None)` unless EXACTLY one root
	/// matches: titles are derived and truncated, so a collision must
	/// refuse rather than attach feedback to the wrong idea. A read
	/// failure is an Err.
	pub fn find_idea_by_exact_title(&self, title: &str) -> Result<Option<IdeaItem>, String> {
		let conn = self.lock();
		let mut stmt = conn
			.prepare(&format!(
				"SELECT {} FROM ideas WHERE title = ?1 AND parent_idea_id IS NULL LIMIT 2",
				Self::IDEA_COLS
			))
			.map_err(|e| format!("failed to look up the idea by title: {e}"))?;
		let mut matches: Vec<IdeaItem> = stmt
			.query_map(params![title], Self::row_to_idea)
			.map_err(|e| format!("failed to look up the idea by title: {e}"))?
			.collect::<Result<Vec<_>, _>>()
			.map_err(|e| format!("failed to look up the idea by title: {e}"))?
			.into_iter()
			.map(|(_, idea)| idea)
			.collect();
		if matches.len() == 1 {
			Ok(Some(matches.remove(0)))
		} else {
			Ok(None)
		}
	}

	/// This idea's share id, if it has one. `Ok(None)` means the row (or
	/// the value) is genuinely absent; a read failure is an Err.
	pub fn get_share_id(&self, id: &str) -> Result<Option<String>, String> {
		let conn = self.lock();
		conn.query_row(
			"SELECT share_id FROM ideas WHERE id = ?1",
			params![id],
			|row| row.get::<_, Option<String>>(0),
		)
		.optional()
		.map_err(|e| format!("failed to read the share id: {e}"))
		.map(Option::flatten)
	}

	/// Lightweight existence + title lookup (no transcript parsing). Used
	/// on hot paths like idea autosave. `Ok(None)` = row missing.
	pub fn get_idea_title(&self, id: &str) -> Result<Option<String>, String> {
		let conn = self.lock();
		conn.query_row(
			"SELECT title FROM ideas WHERE id = ?1",
			params![id],
			|row| row.get::<_, String>(0),
		)
		.optional()
		.map_err(|e| format!("failed to read idea {id}: {e}"))
	}

	pub fn get_idea_children(&self, id: &str) -> Result<Vec<IdeaItem>, String> {
		let conn = self.lock();
		let mut stmt = conn
			.prepare(&format!(
				"SELECT {} FROM ideas WHERE parent_idea_id = ?1 ORDER BY created_at DESC, rowid DESC",
				Self::IDEA_COLS
			))
			.map_err(|e| format!("failed to list feedback: {e}"))?;
		let mut children_items: Vec<IdeaItem> = Vec::new();
		let rows = stmt
			.query_map(params![id], Self::row_to_idea)
			.map_err(|e| format!("failed to list feedback: {e}"))?;
		for row in rows {
			match row {
				Ok((_, idea)) => children_items.push(idea),
				Err(e) => log::error!("skipping unreadable feedback row: {e}"),
			}
		}
		Ok(children_items)
	}

	/// All top-level ideas with their feedback children, newest first, as
	/// summaries (see `row_to_summary`; `get_idea` has the full row).
	/// Single query + one grouping pass (no per-idea child lookups).
	pub fn list_ideas(&self) -> Result<Vec<IdeaItem>, String> {
		let conn = self.lock();
		let mut stmt = conn
			.prepare(&format!(
				"SELECT {} FROM ideas ORDER BY created_at DESC, rowid DESC",
				Self::IDEA_SUMMARY_COLS
			))
			.map_err(|e| format!("failed to list ideas: {e}"))?;
		let mut rows: Vec<(Option<String>, IdeaItem)> = Vec::new();
		{
			let queried = stmt
				.query_map([], Self::row_to_summary)
				.map_err(|e| format!("failed to list ideas: {e}"))?;
			for row in queried {
				match row {
					Ok(item) => rows.push(item),
					// A row that fails to decode must not vanish
					// silently - that reads as "the idea is gone".
					Err(e) => log::error!("skipping unreadable idea row: {e}"),
				}
			}
		}
		drop(stmt);

		let mut children: std::collections::HashMap<String, Vec<IdeaItem>> =
			std::collections::HashMap::new();
		let mut ideas: Vec<IdeaItem> = Vec::new();
		for (parent_id, mut idea) in rows {
			match parent_id {
				Some(parent_id) => {
					children.entry(parent_id).or_default().push(idea);
				}
				None => {
					idea.feedback = Some(Vec::new());
					ideas.push(idea);
				}
			}
		}
		for idea in &mut ideas {
			if let Some(feedback) = children.remove(&idea.id) {
				*idea.feedback.as_mut().unwrap() = feedback;
			}
		}
		Ok(ideas)
	}
}
