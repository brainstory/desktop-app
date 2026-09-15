use std::path::Path;
use std::sync::Mutex;

use chrono::{NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::types::{ChatMessage, DailyStatus, IdeaItem};

pub const DEFAULT_LOG_QUESTIONS: [(i64, &str, &str); 4] = [
	(1, "Did you set an intention for your day?", "Intention"),
	(
		2,
		"Did you think out loud about your ideas today?",
		"Brainstorm",
	),
	(
		3,
		"Did you make progress on yesterday's intention?",
		"Progress",
	),
	(4, "Did you reflect on how your day went?", "Reflection"),
];

pub const SURVEY_QUESTIONS: [&str; 3] = ["focused", "creative", "articulate"];

pub struct Db {
	conn: Mutex<Connection>,
}

fn now_iso() -> String {
	// naive UTC without a trailing Z, matching what the frontend expects
	// (helpers/formatISO8601ToHumanReadable appends the Z itself)
	Utc::now()
		.naive_utc()
		.format("%Y-%m-%dT%H:%M:%S")
		.to_string()
}

fn today_local() -> NaiveDate {
	// date_naive() is infallible; no unwrap needed.
	chrono::Local::now().date_naive()
}

/// The local calendar day a UTC timestamp falls on. Computed at write time
/// so each activity's day is frozen under the timezone it happened in
/// (traveling later must not rewrite history).
fn local_date_for(utc: &str) -> String {
	let date = chrono::NaiveDateTime::parse_from_str(utc, "%Y-%m-%dT%H:%M:%S")
		.ok()
		.map(|naive| {
			chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc)
				.with_timezone(&chrono::Local)
				.date_naive()
		})
		.unwrap_or_else(today_local);
	date.format("%Y-%m-%d").to_string()
}

const BASELINE_SCHEMA: &str = "
	CREATE TABLE IF NOT EXISTS settings (
		key TEXT PRIMARY KEY,
		value TEXT NOT NULL
	);
	CREATE TABLE IF NOT EXISTS ideas (
		id TEXT PRIMARY KEY,
		title TEXT NOT NULL DEFAULT '',
		idea_type TEXT NOT NULL DEFAULT 'original',
		result TEXT NOT NULL DEFAULT '',
		structured_result TEXT,
		transcript TEXT NOT NULL DEFAULT '[]',
		idea_metadata TEXT NOT NULL DEFAULT '{}',
		parent_idea_id TEXT REFERENCES ideas(id) ON DELETE CASCADE,
		log_id TEXT,
		is_unread INTEGER NOT NULL DEFAULT 0,
		creator_name TEXT,
		creator_email TEXT,
		share_id TEXT,
		created_at TEXT NOT NULL
	);
	CREATE TABLE IF NOT EXISTS log_entries (
		id TEXT PRIMARY KEY,
		answers TEXT NOT NULL,
		created_at TEXT NOT NULL
	);
	CREATE TABLE IF NOT EXISTS surveys (
		id TEXT PRIMARY KEY,
		idea_id TEXT,
		answers TEXT NOT NULL,
		created_at TEXT NOT NULL
	);
	CREATE TABLE IF NOT EXISTS daily (
		date TEXT PRIMARY KEY,
		log_id TEXT,
		intent_idea_id TEXT,
		survey_id TEXT,
		is_completed INTEGER NOT NULL DEFAULT 0
	);";

/// Schema history, tracked via `PRAGMA user_version`:
///   1: baseline tables + one-time strip of legacy trailing-Z timestamps
///   2: `daily.created_at` column + lookup indexes
///   3: `local_date` columns on ideas/log_entries/surveys, freezing each
///      activity's local calendar day at write time
/// The baseline also gained `ideas.parent_idea_id REFERENCES ideas(id)
/// ON DELETE CASCADE` before first release; databases from pre-release dev
/// builds simply don't have the constraint (the app-level recursive delete
/// keeps them correct) and are fine to delete and recreate.
/// A database created before this framework exists reports version 0 and is
/// brought forward through every step (the CREATE IF NOT EXISTS statements
/// make step 1 a no-op for its tables).
const SCHEMA_VERSION: i64 = 3;

impl Db {
	pub fn open(path: &Path) -> rusqlite::Result<Self> {
		let conn = Connection::open(path)?;
		conn.pragma_update(None, "journal_mode", "WAL")?;
		// A second app instance (or a stray backup tool) can hold the write
		// lock briefly; wait instead of failing instantly with SQLITE_BUSY.
		conn.pragma_update(None, "busy_timeout", 5000)?;
		// Enforce the parent_idea_id -> ideas(id) foreign key (cascades
		// deletes through the feedback tree at the DB level, backing the
		// app-level recursive delete).
		conn.pragma_update(None, "foreign_keys", "ON")?;

		let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
		if version < 1 {
			conn.execute_batch(BASELINE_SCHEMA)?;
			// Earlier builds stored timestamps with a trailing Z; the
			// frontend appends the Z itself, so strip stored values once.
			conn.execute_batch(
				"UPDATE ideas SET created_at = substr(created_at, 1, 19) WHERE created_at LIKE '%Z';
				 UPDATE log_entries SET created_at = substr(created_at, 1, 19) WHERE created_at LIKE '%Z';
				 UPDATE surveys SET created_at = substr(created_at, 1, 19) WHERE created_at LIKE '%Z';
				 UPDATE settings SET value = substr(value, 1, 19) WHERE key = 'created_at' AND value LIKE '%Z';",
			)?;
		}
		if version < 2 {
			if !Self::table_has_column(&conn, "daily", "created_at")? {
				conn.execute_batch(
					"ALTER TABLE daily ADD COLUMN created_at TEXT NOT NULL DEFAULT '';",
				)?;
			}
			conn.execute_batch(
				"CREATE INDEX IF NOT EXISTS idx_ideas_parent ON ideas(parent_idea_id);
				 CREATE INDEX IF NOT EXISTS idx_ideas_share ON ideas(share_id);
				 CREATE INDEX IF NOT EXISTS idx_daily_intent ON daily(intent_idea_id);",
			)?;
		}
		if version < 3 {
			// Activity days used to be re-derived from UTC timestamps at
			// read time, so history silently shifted after a timezone
			// change while `daily.date` rows stayed frozen. Freeze the
			// local day at write time instead; the backfill freezes
			// existing rows using the current zone (the best guess
			// available for historical data).
			for table in ["ideas", "log_entries", "surveys"] {
				if !Self::table_has_column(&conn, table, "local_date")? {
					conn.execute_batch(&format!(
						"ALTER TABLE {table} ADD COLUMN local_date TEXT NOT NULL DEFAULT '';
						 UPDATE {table} SET local_date = COALESCE(NULLIF(date(created_at, 'localtime'), ''), date('now', 'localtime'));"
					))?;
				}
			}
		}
		conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
		Ok(Self {
			conn: Mutex::new(conn),
		})
	}

	fn table_has_column(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
		let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
		let mut rows = stmt.query([])?;
		while let Some(row) = rows.next()? {
			if row.get::<_, String>("name")? == column {
				return Ok(true);
			}
		}
		Ok(false)
	}

	/// Lock the connection. A poisoned lock still holds a perfectly usable
	/// connection, so recover instead of panicking on every later call.
	fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
		self.conn.lock().unwrap_or_else(|e| e.into_inner())
	}

	/// Run several statements as one transaction; any error rolls back.
	fn with_tx(&self, body: impl FnOnce(&Connection) -> Result<(), String>) -> Result<(), String> {
		let mut conn = self.lock();
		let tx = conn.transaction().map_err(|e| e.to_string())?;
		body(&tx)?;
		tx.commit().map_err(|e| e.to_string())
	}

	// ---- settings ----

	pub fn get_setting(&self, key: &str) -> Option<String> {
		match self
			.lock()
			.query_row(
				"SELECT value FROM settings WHERE key = ?1",
				params![key],
				|row| row.get(0),
			)
			.optional()
		{
			Ok(value) => value.flatten(),
			Err(e) => {
				log::warn!("settings read '{key}' failed: {e}");
				None
			}
		}
	}

	pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
		self.lock()
			.execute(
				"INSERT INTO settings (key, value) VALUES (?1, ?2)
				 ON CONFLICT(key) DO UPDATE SET value = ?2",
				params![key, value],
			)
			.map_err(|e| format!("failed to save setting '{key}': {e}"))?;
		Ok(())
	}

	pub fn set_settings(&self, kv: &[(&str, String)]) -> Result<(), String> {
		self.with_tx(|conn| {
			for (key, value) in kv {
				conn.execute(
					"INSERT INTO settings (key, value) VALUES (?1, ?2)
					 ON CONFLICT(key) DO UPDATE SET value = ?2",
					params![key, value],
				)
				.map_err(|e| format!("failed to save setting '{key}': {e}"))?;
			}
			Ok(())
		})
	}

	/// Remove a setting row entirely (used when secrets move to the
	/// keychain); a no-op if the row doesn't exist.
	pub fn delete_setting(&self, key: &str) {
		if let Err(e) = self
			.lock()
			.execute("DELETE FROM settings WHERE key = ?1", params![key])
		{
			log::warn!("failed to delete setting '{key}': {e}");
		}
	}

	// ---- ideas ----

	/// Split a markdown result into {heading, body} sections for the idea
	/// page's document view (index 0 is the document title, so the feedback
	/// JSON's 1-based heading ordinals line up with array indices).
	fn result_to_json(result: &str) -> serde_json::Value {
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

	const IDEA_COLS: &'static str =
		"id, title, idea_type, result, structured_result, transcript, idea_metadata, parent_idea_id, log_id, is_unread, creator_name, creator_email, share_id, created_at";

	#[allow(clippy::too_many_arguments)]
	fn insert_idea_tx(
		conn: &Connection,
		id: &str,
		title: &str,
		idea_type: &str,
		result: &str,
		structured_result: Option<&serde_json::Value>,
		transcript: &[ChatMessage],
		metadata: &serde_json::Value,
		parent_idea_id: Option<&str>,
		log_id: Option<&str>,
		creator_name: Option<&str>,
		creator_email: Option<&str>,
		share_id: Option<&str>,
		created_at: Option<&str>,
	) -> Result<(), String> {
		let transcript_json = serde_json::to_string(transcript).unwrap_or_else(|_| "[]".into());
		let structured_json = structured_result.map(|v| v.to_string());
		let now = now_iso();
		let created_at = created_at.unwrap_or(&now);
		conn.execute(
			"INSERT INTO ideas (id, title, idea_type, result, structured_result, transcript, idea_metadata, parent_idea_id, log_id, is_unread, creator_name, creator_email, share_id, created_at, local_date)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, ?12, ?13, ?14)",
			params![
				id,
				title,
				idea_type,
				result,
				structured_json,
				transcript_json,
				metadata.to_string(),
				parent_idea_id,
				log_id,
				creator_name,
				creator_email,
				share_id,
				created_at,
				local_date_for(created_at),
			],
		)
		.map_err(|e| format!("failed to save idea: {e}"))?;
		Ok(())
	}

	/// Insert an idea. `created_at` overrides the timestamp (used when
	/// importing shared ideas so they keep their original date). A
	/// `parent_idea_id` is verified inside the same transaction, so a
	/// concurrent delete can't slip an orphan through.
	#[allow(clippy::too_many_arguments)]
	pub fn insert_idea(
		&self,
		id: &str,
		title: &str,
		idea_type: &str,
		result: &str,
		structured_result: Option<&serde_json::Value>,
		transcript: &[ChatMessage],
		metadata: &serde_json::Value,
		parent_idea_id: Option<&str>,
		log_id: Option<&str>,
		creator_name: Option<&str>,
		creator_email: Option<&str>,
		share_id: Option<&str>,
		created_at: Option<&str>,
	) -> Result<(), String> {
		self.with_tx(|conn| {
			if let Some(parent_id) = parent_idea_id {
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
			Self::insert_idea_tx(
				conn,
				id,
				title,
				idea_type,
				result,
				structured_result,
				transcript,
				metadata,
				parent_idea_id,
				log_id,
				creator_name,
				creator_email,
				share_id,
				created_at,
			)
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
		let today = today_local().format("%Y-%m-%d").to_string();
		let mark_completed = !result.is_empty();
		self.with_tx(|conn| {
			Self::insert_idea_tx(
				conn,
				id,
				title,
				"daily_intent",
				result,
				None,
				transcript,
				metadata,
				None,
				None,
				None,
				None,
				None,
				None,
			)?;
			conn.execute(
				"INSERT INTO daily (date, intent_idea_id, is_completed) VALUES (?1, ?2, 0)
				 ON CONFLICT(date) DO UPDATE SET intent_idea_id = ?2",
				params![today, id],
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

	pub fn update_idea(
		&self,
		id: &str,
		title: Option<&str>,
		result: Option<&str>,
		transcript: Option<&[ChatMessage]>,
		structured_result: Option<&serde_json::Value>,
	) -> Result<(), String> {
		self.with_tx(|conn| {
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
			Ok(())
		})
	}

	pub fn mark_idea_read(&self, id: &str) -> Result<(), String> {
		self.lock()
			.execute("UPDATE ideas SET is_unread = 0 WHERE id = ?1", params![id])
			.map_err(|e| format!("failed to mark idea read: {e}"))?;
		Ok(())
	}

	/// Delete an idea and its whole feedback subtree in one transaction
	/// (recursive, so a child-of-child can never survive as an orphan);
	/// also clear any daily-intent reference and survey links so nothing
	/// points at the deleted rows.
	pub fn delete_idea(&self, id: &str) -> Result<bool, String> {
		let mut deleted = false;
		self.with_tx(|conn| {
			conn.execute(
				"UPDATE daily SET intent_idea_id = NULL WHERE intent_idea_id = ?1",
				params![id],
			)
			.map_err(|e| format!("failed to delete idea: {e}"))?;
			conn.execute(
				"UPDATE surveys SET idea_id = NULL WHERE idea_id = ?1",
				params![id],
			)
			.map_err(|e| format!("failed to delete idea: {e}"))?;
			conn.execute(
				"DELETE FROM ideas WHERE id IN (
					WITH RECURSIVE descendants(id) AS (
						SELECT id FROM ideas WHERE parent_idea_id = ?1
						UNION ALL
						SELECT i.id FROM ideas i JOIN descendants d ON i.parent_idea_id = d.id
					)
					SELECT id FROM descendants
				)",
				params![id],
			)
			.map_err(|e| format!("failed to delete feedback: {e}"))?;
			deleted = conn
				.execute("DELETE FROM ideas WHERE id = ?1", params![id])
				.map(|n| n > 0)
				.map_err(|e| format!("failed to delete idea: {e}"))?;
			Ok(())
		})?;
		Ok(deleted)
	}

	pub fn set_idea_unread(&self, id: &str, unread: bool) -> Result<(), String> {
		self.lock()
			.execute(
				"UPDATE ideas SET is_unread = ?1 WHERE id = ?2",
				params![unread as i64, id],
			)
			.map_err(|e| format!("failed to flag idea: {e}"))?;
		Ok(())
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
	/// references an exported idea).
	pub fn get_idea_by_share_id(&self, share_id: &str) -> Option<IdeaItem> {
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
		.ok()
		.flatten()
		.map(|(_, idea)| idea)
	}

	pub fn get_share_id(&self, id: &str) -> Option<String> {
		let conn = self.lock();
		conn.query_row(
			"SELECT share_id FROM ideas WHERE id = ?1",
			params![id],
			|row| row.get(0),
		)
		.optional()
		.ok()
		.flatten()
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

	/// All top-level ideas with their feedback children, newest first.
	/// Single query + one grouping pass (no per-idea child lookups).
	pub fn list_ideas(&self) -> Result<Vec<IdeaItem>, String> {
		let conn = self.lock();
		let mut stmt = conn
			.prepare(&format!(
				"SELECT {} FROM ideas ORDER BY created_at DESC, rowid DESC",
				Self::IDEA_COLS
			))
			.map_err(|e| format!("failed to list ideas: {e}"))?;
		let mut rows: Vec<(Option<String>, IdeaItem)> = Vec::new();
		{
			let queried = stmt
				.query_map([], Self::row_to_idea)
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

	// ---- logs / surveys / daily ----

	pub fn insert_log(
		&self,
		id: &str,
		answers: &[crate::types::LogAnswerItem],
	) -> Result<(), String> {
		let json = serde_json::to_string(answers).unwrap_or_else(|_| "[]".into());
		let today = today_local().format("%Y-%m-%d").to_string();
		self.with_tx(|conn| {
			conn.execute(
				"INSERT INTO log_entries (id, answers, created_at, local_date) VALUES (?1, ?2, ?3, ?4)",
				params![id, json, now_iso(), today],
			)
			.map_err(|e| format!("failed to save daily log: {e}"))?;
			conn.execute(
				"INSERT INTO daily (date, log_id, is_completed) VALUES (?1, ?2, 0)
				 ON CONFLICT(date) DO UPDATE SET log_id = ?2",
				params![today, id],
			)
			.map_err(|e| format!("failed to save daily log: {e}"))?;
			Ok(())
		})
	}

	pub fn get_log_answers_today(&self) -> Option<Vec<crate::types::LogAnswerItem>> {
		let today = today_local().format("%Y-%m-%d").to_string();
		let conn = self.lock();
		let log_id: Option<String> = conn
			.query_row(
				"SELECT log_id FROM daily WHERE date = ?1",
				params![today],
				|row| row.get(0),
			)
			.optional()
			.ok()
			.flatten();
		let log_id = log_id?;
		let answers: String = conn
			.query_row(
				"SELECT answers FROM log_entries WHERE id = ?1",
				params![log_id],
				|row| row.get(0),
			)
			.optional()
			.ok()
			.flatten()?;
		serde_json::from_str(&answers).ok()
	}

	pub fn insert_survey(
		&self,
		id: &str,
		idea_id: Option<&str>,
		answers: &serde_json::Value,
	) -> Result<(), String> {
		let today = today_local().format("%Y-%m-%d").to_string();
		self.with_tx(|conn| {
			conn.execute(
				"INSERT INTO surveys (id, idea_id, answers, created_at, local_date) VALUES (?1, ?2, ?3, ?4, ?5)",
				params![id, idea_id, answers.to_string(), now_iso(), today],
			)
			.map_err(|e| format!("failed to save survey: {e}"))?;
			// Note: a survey does NOT complete the day. is_completed means
			// "today's daily intent was finished" (the dashboard's intent
			// status card reads it); reflecting on the day is recorded via
			// survey_id and counts toward the streak on its own.
			conn.execute(
				"INSERT INTO daily (date, survey_id) VALUES (?1, ?2)
				 ON CONFLICT(date) DO UPDATE SET survey_id = ?2",
				params![today, id],
			)
			.map_err(|e| format!("failed to save survey: {e}"))?;
			Ok(())
		})
	}

	pub fn set_daily_intent(&self, idea_id: &str) -> Result<(), String> {
		let today = today_local().format("%Y-%m-%d").to_string();
		self.lock()
			.execute(
				"INSERT INTO daily (date, intent_idea_id, is_completed) VALUES (?1, ?2, 0)
				 ON CONFLICT(date) DO UPDATE SET intent_idea_id = ?2",
				params![today, idea_id],
			)
			.map_err(|e| format!("failed to save daily intent: {e}"))?;
		Ok(())
	}

	pub fn mark_daily_completed(&self, idea_id: &str) -> Result<(), String> {
		let today = today_local().format("%Y-%m-%d").to_string();
		self.lock()
			.execute(
				"UPDATE daily SET is_completed = 1 WHERE date = ?1 AND intent_idea_id = ?2",
				params![today, idea_id],
			)
			.map_err(|e| format!("failed to complete daily intent: {e}"))?;
		Ok(())
	}

	/// True if any idea, log or survey was recorded today (local time) -
	/// used to skip the daily reminder on already-active days.
	pub fn has_activity_today(&self) -> bool {
		let today = today_local().format("%Y-%m-%d").to_string();
		let conn = self.lock();
		for table in ["ideas", "log_entries", "surveys"] {
			let sql = format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE local_date = ?1)");
			if let Ok(1) = conn.query_row(&sql, params![today], |row| row.get::<_, i64>(0)) {
				return true;
			}
		}
		false
	}

	pub fn get_daily_status(&self) -> DailyStatus {
		let today = today_local().format("%Y-%m-%d").to_string();
		let conn = self.lock();
		let row = conn
			.query_row(
				"SELECT log_id, intent_idea_id, survey_id, is_completed FROM daily WHERE date = ?1",
				params![today],
				|row| {
					Ok((
						row.get::<_, Option<String>>(0)?,
						row.get::<_, Option<String>>(1)?,
						row.get::<_, Option<String>>(2)?,
						row.get::<_, i64>(3)? != 0,
					))
				},
			)
			.optional()
			.ok()
			.flatten();
		let (log_id, intent_idea_id, survey_id, is_completed) =
			row.unwrap_or((None, None, None, false));

		// Streak = consecutive days with any activity (an idea created, a log
		// or survey submitted, or a completed daily intent). Each activity's
		// local calendar day was frozen at write time (local_date), so the
		// history is stable across timezone changes.
		// Collect the raw date strings under the lock, then drop it before
		// parsing so the four scans don't block writers for longer than
		// necessary.
		let mut raw_days: Vec<String> = Vec::new();
		for table in ["ideas", "log_entries", "surveys"] {
			let sql = format!("SELECT DISTINCT local_date FROM {table} WHERE local_date != ''");
			let mut stmt = match conn.prepare(&sql) {
				Ok(s) => s,
				Err(e) => {
					log::error!("streak query on {table} failed: {e}");
					continue;
				}
			};
			let dates: Vec<String> = stmt
				.query_map([], |row| row.get(0))
				.map(|rows| rows.filter_map(|r| r.ok()).collect())
				.unwrap_or_default();
			raw_days.extend(dates);
		}
		match conn.prepare("SELECT date FROM daily WHERE is_completed = 1") {
			Ok(mut stmt) => {
				let days: Vec<String> = stmt
					.query_map([], |row| row.get(0))
					.map(|rows| rows.filter_map(|r| r.ok()).collect())
					.unwrap_or_default();
				raw_days.extend(days);
			}
			Err(e) => log::error!("streak query on daily failed: {e}"),
		}
		drop(conn);

		let mut activity: std::collections::HashSet<NaiveDate> = std::collections::HashSet::new();
		for day in raw_days {
			if let Ok(d) = NaiveDate::parse_from_str(&day, "%Y-%m-%d") {
				activity.insert(d);
			}
		}

		// Walk back from today; if today has no activity yet the streak
		// isn't broken (it continues from yesterday).
		let mut streak = 0i64;
		let mut day = Some(today_local());
		if day.map(|d| !activity.contains(&d)).unwrap_or(true) {
			day = day.and_then(|d| d.pred_opt());
		}
		while let Some(d) = day {
			if !activity.contains(&d) {
				break;
			}
			streak += 1;
			day = d.pred_opt();
		}

		DailyStatus {
			log_id,
			intent_idea_id,
			survey_id,
			is_completed,
			streak,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn temp_db_path() -> std::path::PathBuf {
		std::env::temp_dir().join(format!("brainstory-db-test-{}.db", uuid::Uuid::new_v4()))
	}

	#[test]
	fn survey_does_not_complete_the_daily_intent() {
		let path = temp_db_path();
		let db = Db::open(&path).expect("open");
		db.insert_survey("s1", None, &serde_json::json!({ "focused": 3 }))
			.unwrap();
		let status = db.get_daily_status();
		assert!(
			!status.is_completed,
			"a survey alone must not complete the daily intent"
		);
		assert_eq!(status.survey_id.as_deref(), Some("s1"));
		assert_eq!(status.streak, 1, "the survey still counts as activity");
		std::fs::remove_file(path).ok();
	}

	#[test]
	fn generating_the_intent_result_completes_the_day() {
		let path = temp_db_path();
		let db = Db::open(&path).expect("open");
		// draft intent: recorded but not completed
		db.create_daily_intent_idea("i1", "T", "", &[], &serde_json::json!({}))
			.unwrap();
		assert!(!db.get_daily_status().is_completed);
		// an intent created with its result already present completes the day
		db.create_daily_intent_idea("i2", "T", "## Result", &[], &serde_json::json!({}))
			.unwrap();
		assert!(db.get_daily_status().is_completed);
		std::fs::remove_file(path).ok();
	}

	#[test]
	fn migrates_v2_database_and_backfills_local_date() {
		let path = temp_db_path();
		{
			// a database exactly as schema version 2 left it: daily has
			// created_at, activity tables have no local_date
			let conn = Connection::open(&path).unwrap();
			conn.execute_batch(BASELINE_SCHEMA).unwrap();
			conn.execute_batch("ALTER TABLE daily ADD COLUMN created_at TEXT NOT NULL DEFAULT '';")
				.unwrap();
			conn.execute_batch(
				"INSERT INTO ideas (id, created_at) VALUES ('old', strftime('%Y-%m-%dT%H:%M:%S', 'now', '-1 day'));",
			)
			.unwrap();
			conn.pragma_update(None, "user_version", 2).unwrap();
		}
		let db = Db::open(&path).expect("migrate v2 -> v3");
		{
			let conn = db.lock();
			let version: i64 = conn
				.query_row("PRAGMA user_version", [], |r| r.get(0))
				.unwrap();
			assert_eq!(version, SCHEMA_VERSION);
			let local_date: String = conn
				.query_row("SELECT local_date FROM ideas WHERE id = 'old'", [], |r| {
					r.get(0)
				})
				.unwrap();
			assert_eq!(
				local_date.len(),
				10,
				"backfilled as YYYY-MM-DD: {local_date}"
			);
			assert_ne!(local_date, "", "backfill is not empty");
		}
	// yesterday's idea counts via its frozen local date; today without
	// activity doesn't break the streak
		assert!(db.get_daily_status().streak >= 1);
		std::fs::remove_file(path).ok();
	}

	#[test]
	fn delete_idea_removes_the_whole_feedback_subtree() {
		let path = temp_db_path();
		let db = Db::open(&path).expect("open");
		let meta = serde_json::json!({});
		let empty: Vec<ChatMessage> = vec![];
		db.insert_idea("parent", "P", "original", "r", None, &empty, &meta, None, None, None, None, None, None)
			.unwrap();
		db.insert_idea("child", "C", "feedback", "r", None, &empty, &meta, Some("parent"), None, None, None, None, None)
			.unwrap();
		// a child of the child: the old delete-children-only logic left
		// this row orphaned in the library
		db.insert_idea("grandchild", "G", "feedback", "r", None, &empty, &meta, Some("child"), None, None, None, None, None)
			.unwrap();
		let deleted = db.delete_idea("parent").unwrap();
		assert!(deleted);
		assert!(db.get_idea("parent").unwrap().is_none());
		assert!(db.get_idea("child").unwrap().is_none());
		assert!(db.get_idea("grandchild").unwrap().is_none());
		assert!(db.list_ideas().unwrap().is_empty());
		std::fs::remove_file(path).ok();
	}

	#[test]
	fn insert_idea_rejects_a_missing_parent() {
		let path = temp_db_path();
		let db = Db::open(&path).expect("open");
		let meta = serde_json::json!({});
		let err = db
			.insert_idea("kid", "K", "feedback", "r", None, &[], &meta, Some("ghost"), None, None, None, None, None)
			.expect_err("missing parent must fail");
		assert!(err.contains("not found"), "unexpected error: {err}");
		assert!(db.get_idea("kid").unwrap().is_none(), "nothing inserted");
		std::fs::remove_file(path).ok();
	}

	#[test]
	fn foreign_keys_are_enforced_at_the_schema_level() {
		let path = temp_db_path();
		let db = Db::open(&path).expect("open");
		{
			let conn = db.lock();
			// a direct SQL insert bypassing the app's checks must hit the FK
			let ghost = conn.execute(
				"INSERT INTO ideas (id, created_at, parent_idea_id) VALUES ('orphan', '2026-01-01T00:00:00', 'ghost')",
				[],
			);
			assert!(ghost.is_err(), "FK must reject a missing parent");

			// ON DELETE CASCADE: removing the parent removes the subtree
			conn.execute("INSERT INTO ideas (id, created_at) VALUES ('p', '2026-01-01T00:00:00')", []).unwrap();
			conn.execute(
				"INSERT INTO ideas (id, created_at, parent_idea_id) VALUES ('c', '2026-01-01T00:00:00', 'p')",
				[],
			)
			.unwrap();
			conn.execute("DELETE FROM ideas WHERE id = 'p'", []).unwrap();
			let orphans: i64 = conn
				.query_row(
					"SELECT COUNT(*) FROM ideas WHERE id = 'c'",
					[],
					|row| row.get(0),
				)
				.unwrap();
			assert_eq!(orphans, 0, "cascade must remove the child");
		}
		std::fs::remove_file(path).ok();
	}

	#[test]
	fn get_idea_title_is_a_lightweight_read() {
		let path = temp_db_path();
		let db = Db::open(&path).expect("open");
		let meta = serde_json::json!({});
		db.insert_idea("t1", "The Title", "original", "r", None, &[], &meta, None, None, None, None, None, None)
			.unwrap();
		assert_eq!(db.get_idea_title("t1").unwrap().as_deref(), Some("The Title"));
		assert_eq!(db.get_idea_title("missing").unwrap(), None);
		std::fs::remove_file(path).ok();
	}
}
