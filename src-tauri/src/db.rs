use std::path::Path;
use std::sync::Mutex;

use chrono::{Datelike, NaiveDate, Utc};
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
	let local = chrono::Local::now();
	NaiveDate::from_ymd_opt(local.year(), local.month(), local.day()).unwrap()
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
		parent_idea_id TEXT,
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
/// A database created before this framework exists reports version 0 and is
/// brought forward through every step (the CREATE IF NOT EXISTS statements
/// make step 1 a no-op for its tables).
const SCHEMA_VERSION: i64 = 2;

impl Db {
	pub fn open(path: &Path) -> rusqlite::Result<Self> {
		let conn = Connection::open(path)?;
		conn.pragma_update(None, "journal_mode", "WAL")?;
		// A second app instance (or a stray backup tool) can hold the write
		// lock briefly; wait instead of failing instantly with SQLITE_BUSY.
		conn.pragma_update(None, "busy_timeout", 5000)?;

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
		conn.execute(
			"INSERT INTO ideas (id, title, idea_type, result, structured_result, transcript, idea_metadata, parent_idea_id, log_id, is_unread, creator_name, creator_email, share_id, created_at)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, ?12, ?13)",
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
				created_at.unwrap_or(&now_iso()),
			],
		)
		.map_err(|e| format!("failed to save idea: {e}"))?;
		Ok(())
	}

	/// Insert an idea. `created_at` overrides the timestamp (used when
	/// importing shared ideas so they keep their original date).
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

	/// Delete an idea and its feedback children in one transaction; also
	/// clear any daily-intent reference and survey links so nothing points
	/// at the deleted rows.
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
			conn.execute("DELETE FROM ideas WHERE parent_idea_id = ?1", params![id])
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

	pub fn get_idea(&self, id: &str) -> Option<IdeaItem> {
		let conn = self.lock();
		let (parent_id, mut idea) = conn
			.query_row(
				&format!("SELECT {} FROM ideas WHERE id = ?1", Self::IDEA_COLS),
				params![id],
				Self::row_to_idea,
			)
			.optional()
			.ok()
			.flatten()?;

		if let Some(parent_id) = parent_id {
			if let Some((_, parent)) = conn
				.query_row(
					&format!("SELECT {} FROM ideas WHERE id = ?1", Self::IDEA_COLS),
					params![parent_id],
					Self::row_to_idea,
				)
				.optional()
				.ok()
				.flatten()
			{
				idea.parent_idea = Some(Box::new(parent));
			}
		}
		Some(idea)
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

	pub fn get_idea_children(&self, id: &str) -> Vec<IdeaItem> {
		let conn = self.lock();
		let mut stmt = match conn.prepare(&format!(
			"SELECT {} FROM ideas WHERE parent_idea_id = ?1 ORDER BY created_at DESC, rowid DESC",
			Self::IDEA_COLS
		)) {
			Ok(s) => s,
			Err(e) => {
				log::warn!("failed to list feedback: {e}");
				return vec![];
			}
		};
		stmt.query_map(params![id], Self::row_to_idea)
			.map(|rows| rows.filter_map(|r| r.ok().map(|(_, idea)| idea)).collect())
			.unwrap_or_default()
	}

	/// All top-level ideas with their feedback children, newest first.
	/// Single query + one grouping pass (no per-idea child lookups).
	pub fn list_ideas(&self) -> Vec<IdeaItem> {
		let conn = self.lock();
		let mut stmt = match conn.prepare(&format!(
			"SELECT {} FROM ideas ORDER BY created_at DESC, rowid DESC",
			Self::IDEA_COLS
		)) {
			Ok(s) => s,
			Err(e) => {
				log::warn!("failed to list ideas: {e}");
				return vec![];
			}
		};
		let rows: Vec<(Option<String>, IdeaItem)> = stmt
			.query_map([], Self::row_to_idea)
			.map(|rows| rows.filter_map(|r| r.ok()).collect())
			.unwrap_or_default();
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
		ideas
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
				"INSERT INTO log_entries (id, answers, created_at) VALUES (?1, ?2, ?3)",
				params![id, json, now_iso()],
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
				"INSERT INTO surveys (id, idea_id, answers, created_at) VALUES (?1, ?2, ?3, ?4)",
				params![id, idea_id, answers.to_string(), now_iso()],
			)
			.map_err(|e| format!("failed to save survey: {e}"))?;
			// Note: submitting the end-of-day survey marks the day completed.
			// This is deliberate - reflecting on the day closes it out.
			conn.execute(
				"INSERT INTO daily (date, survey_id, is_completed) VALUES (?1, ?2, 1)
				 ON CONFLICT(date) DO UPDATE SET survey_id = ?2, is_completed = 1",
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
		let conn = self.lock();
		for table in ["ideas", "log_entries", "surveys"] {
			let sql = format!(
				"SELECT EXISTS(SELECT 1 FROM {table} WHERE date(created_at, 'localtime') = date('now', 'localtime'))"
			);
			if let Ok(1) = conn.query_row(&sql, [], |row| row.get::<_, i64>(0)) {
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
		// or survey submitted, or a completed daily intent). Timestamps are
		// stored in UTC; SQLite's 'localtime' modifier converts them to the
		// local calendar day (same tz database chrono::Local uses), so late
		// evening sessions land on the correct day.
		let mut activity: std::collections::HashSet<NaiveDate> = std::collections::HashSet::new();
		for table in ["ideas", "log_entries", "surveys"] {
			let sql = format!("SELECT DISTINCT date(created_at, 'localtime') FROM {table}");
			let mut stmt = match conn.prepare(&sql) {
				Ok(s) => s,
				Err(_) => continue,
			};
			let dates: Vec<String> = stmt
				.query_map([], |row| row.get(0))
				.map(|rows| rows.filter_map(|r| r.ok()).collect())
				.unwrap_or_default();
			for day in dates {
				if let Ok(d) = NaiveDate::parse_from_str(&day, "%Y-%m-%d") {
					activity.insert(d);
				}
			}
		}
		{
			let mut stmt = match conn.prepare("SELECT date FROM daily WHERE is_completed = 1") {
				Ok(s) => s,
				Err(_) => {
					return DailyStatus {
						log_id,
						intent_idea_id,
						survey_id,
						is_completed,
						streak: 0,
					}
				}
			};
			let days: Vec<String> = stmt
				.query_map([], |row| row.get(0))
				.map(|rows| rows.filter_map(|r| r.ok()).collect())
				.unwrap_or_default();
			for day in days {
				if let Ok(d) = NaiveDate::parse_from_str(&day, "%Y-%m-%d") {
					activity.insert(d);
				}
			}
		}
		drop(conn);

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
