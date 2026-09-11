use std::path::Path;
use std::sync::Mutex;

use chrono::{Datelike, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::types::{ChatMessage, DailyStatus, IdeaItem};

pub const DEFAULT_LOG_QUESTIONS: [(i64, &str, &str); 4] = [
	(1, "Did you set an intention for your day?", "Intention"),
	(2, "Did you think out loud about your ideas today?", "Brainstorm"),
	(3, "Did you make progress on yesterday's intention?", "Progress"),
	(4, "Did you reflect on how your day went?", "Reflection"),
];

pub const SURVEY_QUESTIONS: [&str; 3] = ["focused", "creative", "articulate"];

pub struct Db {
	conn: Mutex<Connection>,
}

fn now_iso() -> String {
	// naive UTC without a trailing Z, matching what the frontend expects
	// (helpers/formatISO8601ToHumanReadable appends the Z itself)
	Utc::now().naive_utc().format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn today_local() -> NaiveDate {
	let local = chrono::Local::now();
	NaiveDate::from_ymd_opt(local.year(), local.month(), local.day()).unwrap()
}

impl Db {
	pub fn open(path: &Path) -> rusqlite::Result<Self> {
		let conn = Connection::open(path)?;
		conn.pragma_update(None, "journal_mode", "WAL")?;
		conn.execute_batch(
			"CREATE TABLE IF NOT EXISTS settings (
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
			);",
		)?;
		// Earlier builds stored timestamps with a trailing Z; the frontend
		// appends the Z itself, so strip it from anything already stored.
		conn.execute_batch(
			"UPDATE ideas SET created_at = substr(created_at, 1, 19) WHERE created_at LIKE '%Z';
			 UPDATE log_entries SET created_at = substr(created_at, 1, 19) WHERE created_at LIKE '%Z';
			 UPDATE surveys SET created_at = substr(created_at, 1, 19) WHERE created_at LIKE '%Z';
			 UPDATE settings SET value = substr(value, 1, 19) WHERE key = 'created_at' AND value LIKE '%Z';",
		)?;
		Ok(Self { conn: Mutex::new(conn) })
	}

	// ---- settings ----

	pub fn get_setting(&self, key: &str) -> Option<String> {
		self.conn
			.lock()
			.unwrap()
			.query_row(
				"SELECT value FROM settings WHERE key = ?1",
				params![key],
				|row| row.get(0),
			)
			.optional()
			.ok()
			.flatten()
	}

	pub fn set_setting(&self, key: &str, value: &str) {
		self.conn
			.lock()
			.unwrap()
			.execute(
				"INSERT INTO settings (key, value) VALUES (?1, ?2)
				 ON CONFLICT(key) DO UPDATE SET value = ?2",
				params![key, value],
			)
			.ok();
	}

	pub fn set_settings(&self, kv: &[(&str, String)]) {
		let conn = self.conn.lock().unwrap();
		for (key, value) in kv {
			conn.execute(
				"INSERT INTO settings (key, value) VALUES (?1, ?2)
				 ON CONFLICT(key) DO UPDATE SET value = ?2",
				params![key, value],
			)
			.ok();
		}
	}

	// ---- ideas ----

	fn row_to_idea(row: &rusqlite::Row) -> rusqlite::Result<IdeaItem> {
		let transcript: String = row.get("transcript")?;
		let structured: Option<String> = row.get("structured_result")?;
		Ok(IdeaItem {
			id: row.get("id")?,
			title: row.get("title")?,
			result: Some(row.get::<_, String>("result")?),
			r#type: Some(row.get::<_, String>("idea_type")?),
			created_at: row.get("created_at")?,
			creator_email: row.get("creator_email")?,
			creator_name: row.get("creator_name")?,
			is_unread: Some(row.get::<_, i64>("is_unread")? != 0),
			transcript: serde_json::from_str::<Vec<ChatMessage>>(&transcript).ok(),
			shared_with_users: None,
			structured_result: structured.and_then(|s| serde_json::from_str(&s).ok()),
			result_json: None,
			parent_idea: None,
			feedback: None,
		})
	}

	const IDEA_COLS: &'static str =
		"id, title, idea_type, result, structured_result, transcript, idea_metadata, parent_idea_id, log_id, is_unread, creator_name, creator_email, share_id, created_at";

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
	) {
		let transcript_json = serde_json::to_string(transcript).unwrap_or_else(|_| "[]".into());
		let structured_json = structured_result.map(|v| v.to_string());
		self.conn
			.lock()
			.unwrap()
			.execute(
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
					now_iso()
				],
			)
			.ok();
	}

	pub fn update_idea(
		&self,
		id: &str,
		title: Option<&str>,
		result: Option<&str>,
		transcript: Option<&[ChatMessage]>,
		structured_result: Option<&serde_json::Value>,
	) {
		let conn = self.conn.lock().unwrap();
		if let Some(t) = title {
			conn.execute("UPDATE ideas SET title = ?1 WHERE id = ?2", params![t, id]).ok();
		}
		if let Some(r) = result {
			conn.execute("UPDATE ideas SET result = ?1 WHERE id = ?2", params![r, id]).ok();
		}
		if let Some(t) = transcript {
			let json = serde_json::to_string(t).unwrap_or_else(|_| "[]".into());
			conn.execute("UPDATE ideas SET transcript = ?1 WHERE id = ?2", params![json, id]).ok();
		}
		if let Some(s) = structured_result {
			conn.execute(
				"UPDATE ideas SET structured_result = ?1 WHERE id = ?2",
				params![s.to_string(), id],
			)
			.ok();
		}
	}

	pub fn mark_idea_read(&self, id: &str) {
		self.conn
			.lock()
			.unwrap()
			.execute("UPDATE ideas SET is_unread = 0 WHERE id = ?1", params![id])
			.ok();
	}

	/// Delete an idea and its feedback children; also clear any daily-intent
	/// reference so the dashboard doesn't link to a missing idea.
	pub fn delete_idea(&self, id: &str) -> bool {
		let conn = self.conn.lock().unwrap();
		conn.execute(
			"UPDATE daily SET intent_idea_id = NULL WHERE intent_idea_id = ?1",
			params![id],
		)
		.ok();
		conn.execute(
			"DELETE FROM ideas WHERE parent_idea_id = ?1",
			params![id],
		)
		.ok();
		conn.execute("DELETE FROM ideas WHERE id = ?1", params![id])
			.map(|n| n > 0)
			.unwrap_or(false)
	}

	pub fn set_idea_unread(&self, id: &str, unread: bool) {
		self.conn
			.lock()
			.unwrap()
			.execute(
				"UPDATE ideas SET is_unread = ?1 WHERE id = ?2",
				params![unread as i64, id],
			)
			.ok();
	}

	pub fn get_idea(&self, id: &str) -> Option<IdeaItem> {
		let conn = self.conn.lock().unwrap();
		let mut idea = conn
			.query_row(
				&format!("SELECT {} FROM ideas WHERE id = ?1", Self::IDEA_COLS),
				params![id],
				Self::row_to_idea,
			)
			.optional()
			.ok()
			.flatten()?;

		if let Some(parent_id) = conn
			.query_row(
				"SELECT parent_idea_id FROM ideas WHERE id = ?1",
				params![id],
				|row| row.get::<_, Option<String>>(0),
			)
			.optional()
			.ok()
			.flatten()
			.flatten()
		{
			if let Some(parent) = conn
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
		let conn = self.conn.lock().unwrap();
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
	}

	pub fn get_share_id(&self, id: &str) -> Option<String> {
		let conn = self.conn.lock().unwrap();
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
		let conn = self.conn.lock().unwrap();
		let mut stmt = match conn.prepare(&format!(
			"SELECT {} FROM ideas WHERE parent_idea_id = ?1 ORDER BY created_at DESC",
			Self::IDEA_COLS
		)) {
			Ok(s) => s,
			Err(_) => return vec![],
		};
		stmt.query_map(params![id], Self::row_to_idea)
			.map(|rows| rows.filter_map(|r| r.ok()).collect())
			.unwrap_or_default()
	}

	pub fn list_ideas(&self) -> Vec<IdeaItem> {
		let conn = self.conn.lock().unwrap();
		let mut stmt = match conn.prepare(&format!(
			"SELECT {} FROM ideas WHERE parent_idea_id IS NULL ORDER BY created_at DESC",
			Self::IDEA_COLS
		)) {
			Ok(s) => s,
			Err(_) => return vec![],
		};
		let mut ideas: Vec<IdeaItem> = stmt
			.query_map([], Self::row_to_idea)
			.map(|rows| rows.filter_map(|r| r.ok()).collect())
			.unwrap_or_default();
		drop(stmt);
		for idea in &mut ideas {
			let mut stmt = match conn.prepare(&format!(
				"SELECT {} FROM ideas WHERE parent_idea_id = ?1 ORDER BY created_at DESC",
				Self::IDEA_COLS
			)) {
				Ok(s) => s,
				Err(_) => continue,
			};
			idea.feedback = Some(
				stmt.query_map(params![idea.id], Self::row_to_idea)
					.map(|rows| rows.filter_map(|r| r.ok()).collect())
					.unwrap_or_default(),
			);
		}
		ideas
	}

	// ---- logs / surveys / daily ----

	pub fn insert_log(&self, id: &str, answers: &[crate::types::LogAnswerItem]) {
		let json = serde_json::to_string(answers).unwrap_or_else(|_| "[]".into());
		let conn = self.conn.lock().unwrap();
		conn.execute(
			"INSERT INTO log_entries (id, answers, created_at) VALUES (?1, ?2, ?3)",
			params![id, json, now_iso()],
		)
		.ok();
		let today = today_local().format("%Y-%m-%d").to_string();
		conn.execute(
			"INSERT INTO daily (date, log_id, is_completed) VALUES (?1, ?2, 0)
			 ON CONFLICT(date) DO UPDATE SET log_id = ?2",
			params![today, id],
		)
		.ok();
	}

	pub fn get_log_answers_today(&self) -> Option<Vec<crate::types::LogAnswerItem>> {
		let today = today_local().format("%Y-%m-%d").to_string();
		let conn = self.conn.lock().unwrap();
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

	pub fn insert_survey(&self, id: &str, idea_id: Option<&str>, answers: &serde_json::Value) {
		let conn = self.conn.lock().unwrap();
		conn.execute(
			"INSERT INTO surveys (id, idea_id, answers, created_at) VALUES (?1, ?2, ?3, ?4)",
			params![id, idea_id, answers.to_string(), now_iso()],
		)
		.ok();
		let today = today_local().format("%Y-%m-%d").to_string();
		conn.execute(
			"INSERT INTO daily (date, survey_id, is_completed) VALUES (?1, ?2, 1)
			 ON CONFLICT(date) DO UPDATE SET survey_id = ?2, is_completed = 1",
			params![today, id],
		)
		.ok();
	}

	pub fn set_daily_intent(&self, idea_id: &str) {
		let today = today_local().format("%Y-%m-%d").to_string();
		self.conn
			.lock()
			.unwrap()
			.execute(
				"INSERT INTO daily (date, intent_idea_id, is_completed) VALUES (?1, ?2, 0)
				 ON CONFLICT(date) DO UPDATE SET intent_idea_id = ?2",
				params![today, idea_id],
			)
			.ok();
	}

	pub fn mark_daily_completed(&self, idea_id: &str) {
		let today = today_local().format("%Y-%m-%d").to_string();
		self.conn
			.lock()
			.unwrap()
			.execute(
				"UPDATE daily SET is_completed = 1 WHERE date = ?1 AND intent_idea_id = ?2",
				params![today, idea_id],
			)
			.ok();
	}

	pub fn get_daily_status(&self) -> DailyStatus {
		let today = today_local().format("%Y-%m-%d").to_string();
		let conn = self.conn.lock().unwrap();
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

		// streak: count consecutive completed days walking back from today.
		// today not being complete yet doesn't break the streak.
		let mut streak = 0i64;
		let mut day = today_local();
		if !is_completed {
			day = day.pred_opt().unwrap_or(day);
		}
		for _ in 0..3660 {
			let key = day.format("%Y-%m-%d").to_string();
			let done: Option<bool> = conn
				.query_row(
					"SELECT is_completed FROM daily WHERE date = ?1",
					params![key],
					|row| Ok(row.get::<_, i64>(0)? != 0),
				)
				.optional()
				.ok()
				.flatten();
			match done {
				Some(true) => {
					streak += 1;
					day = day.pred_opt().unwrap_or(day);
				}
				_ => break,
			}
		}

		DailyStatus { log_id, intent_idea_id, survey_id, is_completed, streak }
	}

	pub fn get_daily_list(&self) -> Vec<(String, Option<String>, Option<String>, String)> {
		let conn = self.conn.lock().unwrap();
		let mut stmt = match conn.prepare(
			"SELECT date, intent_idea_id, survey_id, created_at FROM daily
			 WHERE intent_idea_id IS NOT NULL ORDER BY date DESC LIMIT 120",
		) {
			Ok(s) => s,
			Err(_) => return vec![],
		};
		stmt.query_map([], |row| {
			Ok((
				row.get::<_, String>(0)?,
				row.get::<_, Option<String>>(1)?,
				row.get::<_, Option<String>>(2)?,
				row.get::<_, String>(3)?,
			))
		})
		.map(|rows| rows.filter_map(|r| r.ok()).collect())
		.unwrap_or_default()
	}
}
