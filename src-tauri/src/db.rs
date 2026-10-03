use std::path::Path;
use std::sync::Mutex;

use chrono::{NaiveDate, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension};

use crate::keys::setting::USER_TIMEZONE;
use crate::reactions::{
	validate_reaction, CommentReaction, IdeaReactions, SectionReaction, SharedSectionReaction,
};
use crate::types::{ChatMessage, DailyStatus, IdeaItem, IdeaType};

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
	/// The resolved user timezone (None = OS-local). An in-memory mirror
	/// of the stored `user_timezone` row, read at open and refreshed by
	/// the settings save path; `today_local`/`local_date_for` and the
	/// reminder scheduler compute day boundaries from it. Never locked
	/// while the connection lock is held the other way around (a zone
	/// guard is never held across a database operation).
	zone: Mutex<Option<Tz>>,
}

/// Failure to open (and migrate) the database file.
#[derive(Debug)]
pub enum OpenError {
	/// SQLite itself failed (I/O error, corruption, ...).
	Sqlite(rusqlite::Error),
	/// The file was written by a newer Brainstory build. Refuse to touch
	/// it instead of misreading newer rows or stamping an older
	/// `user_version` over it (which would re-run migrations against an
	/// already-migrated schema on the next launch).
	NewerSchema { found: i64, supported: i64 },
}

impl std::fmt::Display for OpenError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Sqlite(e) => write!(f, "{e}"),
			Self::NewerSchema { found, supported } => write!(
				f,
				"this database was created by a newer version of Brainstory (schema version {found}, this build supports {supported}) - update Brainstory to open it"
			),
		}
	}
}

impl std::error::Error for OpenError {}

impl From<rusqlite::Error> for OpenError {
	fn from(e: rusqlite::Error) -> Self {
		Self::Sqlite(e)
	}
}

fn now_iso() -> String {
	// naive UTC without a trailing Z, matching what the frontend expects
	// (helpers/formatISO8601ToHumanReadable appends the Z itself)
	Utc::now()
		.naive_utc()
		.format("%Y-%m-%dT%H:%M:%S")
		.to_string()
}

/// Resolve a stored `user_timezone` value to a chrono-tz zone. Empty
/// and unknown names (a hand-edited row, a zone this build's embedded
/// tz database does not know) fall back to None = OS-local, which is
/// exactly the behavior before the setting became authoritative.
pub(crate) fn parse_zone(value: &str) -> Option<Tz> {
	if value.is_empty() {
		return None;
	}
	value.parse::<Tz>().ok()
}

/// Today's calendar date in the active zone.
fn today_in(zone: Option<Tz>) -> NaiveDate {
	match zone {
		Some(tz) => Utc::now().with_timezone(&tz).date_naive(),
		None => chrono::Local::now().date_naive(),
	}
}

/// The local calendar day a UTC timestamp falls on, in the active zone.
/// Computed at write time so each activity's day is frozen under the
/// timezone it happened in (a later zone change must not rewrite
/// history - only future writes use the new zone).
fn local_date_for(utc: &str, zone: Option<Tz>) -> String {
	let date = chrono::NaiveDateTime::parse_from_str(utc, "%Y-%m-%dT%H:%M:%S")
		.ok()
		.map(|naive| {
			let utc_dt =
				chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc);
			match zone {
				Some(tz) => utc_dt.with_timezone(&tz).date_naive(),
				None => utc_dt.with_timezone(&chrono::Local).date_naive(),
			}
		})
		.unwrap_or_else(|| today_in(zone));
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

/// Schema step 5. One section reaction per (idea, section, emoji, source):
/// the IFNULL folds every local (NULL-source) reaction into one key, which
/// a plain UNIQUE constraint would not (NULLs never compare equal).
const REACTIONS_SCHEMA: &str = "
	CREATE TABLE IF NOT EXISTS section_reactions (
		id INTEGER PRIMARY KEY,
		idea_id TEXT NOT NULL REFERENCES ideas(id) ON DELETE CASCADE,
		section_index INTEGER NOT NULL CHECK (section_index >= 0),
		emoji TEXT NOT NULL,
		source_idea_id TEXT REFERENCES ideas(id) ON DELETE CASCADE,
		created_at TEXT NOT NULL
	);
	CREATE UNIQUE INDEX IF NOT EXISTS idx_section_reactions_unique
		ON section_reactions(idea_id, section_index, emoji, IFNULL(source_idea_id, ''));
	CREATE INDEX IF NOT EXISTS idx_section_reactions_source ON section_reactions(source_idea_id);
	CREATE TABLE IF NOT EXISTS comment_reactions (
		id INTEGER PRIMARY KEY,
		feedback_idea_id TEXT NOT NULL REFERENCES ideas(id) ON DELETE CASCADE,
		item_index INTEGER NOT NULL CHECK (item_index >= 0),
		emoji TEXT NOT NULL,
		created_at TEXT NOT NULL,
		UNIQUE (feedback_idea_id, item_index, emoji)
	);";

/// Schema history, tracked via `PRAGMA user_version`:
///   1: baseline tables + one-time strip of legacy trailing-Z timestamps
///   2: `daily.created_at` column + lookup indexes
///   3: `local_date` columns on ideas/log_entries/surveys, freezing each
///      activity's local calendar day at write time
///   4: indexes on the local_date columns (streak and activity-today
///      queries scan them on every dashboard load)
///   5: `section_reactions` and `comment_reactions` (emoji reactions
///      people put on idea sections and feedback comments)
/// The baseline also gained `ideas.parent_idea_id REFERENCES ideas(id)
/// ON DELETE CASCADE` before first release; databases from pre-release dev
/// builds simply don't have the constraint (the app-level recursive delete
/// keeps them correct) and are fine to delete and recreate.
/// A database created before this framework exists reports version 0 and is
/// brought forward through every step (the CREATE IF NOT EXISTS statements
/// make step 1 a no-op for its tables).
///
/// Column notes:
/// - `daily.created_at`: when the day's row was first written (naive UTC,
///   like every created_at); rows written before it was populated hold ''.
/// - `ideas.idea_metadata`: free-form JSON from the creator; read by the
///   backend only for `imported` (share-file rows, excluded from the
///   streak and activity queries).
/// - `ideas.log_id`: the daily log an idea was created from, written for
///   provenance; nothing reads it yet.
/// - `section_reactions.section_index`: index into the idea's sections in
///   `Db::result_to_json` order. `source_idea_id` NULL = the local user's
///   own reaction; otherwise the imported feedback idea that carried it
///   (deleting that feedback deletes its reactions).
/// - `comment_reactions.item_index`: index into the feedback idea's
///   `structured_result.feedback_items`; the idea author's, never shared.
const SCHEMA_VERSION: i64 = 5;

/// SQL predicate matching idea rows that came from a share file
/// (`idea_metadata.imported = true`). Imported rows never count as the
/// importer's own activity, so the streak/activity queries exclude them
/// and the local_date backfill leaves them alone. The CASE guards
/// json_extract, which raises on a malformed metadata string.
const IMPORTED_IDEA_SQL: &str =
	"CASE WHEN json_valid(idea_metadata) THEN json_extract(idea_metadata, '$.imported') END IS 1";

/// Extra WHERE clause that keeps imported rows out of activity scans and
/// the local_date backfill (only `ideas` carries imports).
fn own_activity_filter(table: &str) -> String {
	if table == "ideas" {
		format!(" AND NOT ({IMPORTED_IDEA_SQL})")
	} else {
		String::new()
	}
}

/// Everything needed to insert one idea row. Replaces a 13-parameter
/// positional signature where every argument was a bare &str/Option.
#[derive(Default)]
pub struct NewIdea<'a> {
	pub id: &'a str,
	pub title: &'a str,
	pub idea_type: IdeaType,
	pub result: &'a str,
	pub structured_result: Option<&'a serde_json::Value>,
	pub transcript: &'a [ChatMessage],
	pub metadata: &'a serde_json::Value,
	pub parent_idea_id: Option<&'a str>,
	pub log_id: Option<&'a str>,
	pub creator_name: Option<&'a str>,
	pub creator_email: Option<&'a str>,
	pub share_id: Option<&'a str>,
	/// overrides the timestamp (imports keep their original date)
	pub created_at: Option<&'a str>,
	pub imported: bool,
	/// Section reactions this (imported feedback) idea carries onto its
	/// parent idea; stored in the same transaction, attributed to this
	/// idea. Requires `parent_idea_id`.
	pub parent_section_reactions: &'a [SharedSectionReaction],
}

impl<'a> NewIdea<'a> {
	/// The share-file variant: keeps `created_at`, never counts toward
	/// the importer's own activity.
	pub fn imported(idea: NewIdea<'a>) -> Self {
		Self {
			imported: true,
			..idea
		}
	}
}

impl Db {
	pub fn open(path: &Path) -> Result<Self, OpenError> {
		let mut conn = Connection::open(path)?;
		conn.pragma_update(None, "journal_mode", "WAL")?;
		// A second app instance (or a stray backup tool) can hold the write
		// lock briefly; wait instead of failing instantly with SQLITE_BUSY.
		conn.pragma_update(None, "busy_timeout", 5000)?;
		// Enforce the parent_idea_id -> ideas(id) foreign key (cascades
		// deletes through the feedback tree at the DB level, backing the
		// app-level recursive delete).
		conn.pragma_update(None, "foreign_keys", "ON")?;

		let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
		if version > SCHEMA_VERSION {
			// Never migrate (or stamp) a database from a newer build: this
			// build does not know what its schema looks like. Leave the
			// file exactly as found for the newer version to reopen.
			return Err(OpenError::NewerSchema {
				found: version,
				supported: SCHEMA_VERSION,
			});
		}
		// Each step is one transaction committed together with its
		// user_version bump: execute_batch statements auto-commit
		// individually, so a crash mid-step used to leave the schema half
		// migrated with the version stamp making it permanent.
		if version < 1 {
			let tx = conn.transaction()?;
			tx.execute_batch(BASELINE_SCHEMA)?;
			// Earlier builds stored timestamps with a trailing Z; the
			// frontend appends the Z itself, so strip stored values once.
			tx.execute_batch(
				"UPDATE ideas SET created_at = substr(created_at, 1, 19) WHERE created_at LIKE '%Z';
				 UPDATE log_entries SET created_at = substr(created_at, 1, 19) WHERE created_at LIKE '%Z';
				 UPDATE surveys SET created_at = substr(created_at, 1, 19) WHERE created_at LIKE '%Z';
				 UPDATE settings SET value = substr(value, 1, 19) WHERE key = 'created_at' AND value LIKE '%Z';",
			)?;
			tx.pragma_update(None, "user_version", 1)?;
			tx.commit()?;
		}
		if version < 2 {
			let tx = conn.transaction()?;
			if !Self::table_has_column(&tx, "daily", "created_at")? {
				tx.execute_batch(
					"ALTER TABLE daily ADD COLUMN created_at TEXT NOT NULL DEFAULT '';",
				)?;
			}
			tx.execute_batch(
				"CREATE INDEX IF NOT EXISTS idx_ideas_parent ON ideas(parent_idea_id);
				 CREATE INDEX IF NOT EXISTS idx_ideas_share ON ideas(share_id);
				 CREATE INDEX IF NOT EXISTS idx_daily_intent ON daily(intent_idea_id);",
			)?;
			tx.pragma_update(None, "user_version", 2)?;
			tx.commit()?;
		}
		if version < 3 {
			// Activity days used to be re-derived from UTC timestamps at
			// read time, so history silently shifted after a timezone
			// change while `daily.date` rows stayed frozen. Freeze the
			// local day at write time instead; the backfill freezes
			// existing rows using the current zone (the best guess
			// available for historical data).
			let tx = conn.transaction()?;
			for table in ["ideas", "log_entries", "surveys"] {
				if !Self::table_has_column(&tx, table, "local_date")? {
					tx.execute_batch(&format!(
						"ALTER TABLE {table} ADD COLUMN local_date TEXT NOT NULL DEFAULT '';"
					))?;
				}
				// The backfill re-runs (not just with the ALTER): the old
				// code skipped it entirely when the column already existed
				// from a crash between the two statements.
				tx.execute_batch(&format!(
					"UPDATE {table} SET local_date = COALESCE(NULLIF(date(created_at, 'localtime'), ''), date('now', 'localtime')) WHERE local_date = ''{};",
					own_activity_filter(table)
				))?;
			}
			tx.pragma_update(None, "user_version", 3)?;
			tx.commit()?;
		}
		if version < 4 {
			// The streak and has_activity_today queries filter on
			// local_date across three tables on every dashboard load.
			let tx = conn.transaction()?;
			tx.execute_batch(
				"CREATE INDEX IF NOT EXISTS idx_ideas_local_date ON ideas(local_date);
				 CREATE INDEX IF NOT EXISTS idx_log_entries_local_date ON log_entries(local_date);
				 CREATE INDEX IF NOT EXISTS idx_surveys_local_date ON surveys(local_date);",
			)?;
			tx.pragma_update(None, "user_version", 4)?;
			tx.commit()?;
		}
		if version < 5 {
			// Emoji reactions chosen by people. Both tables cascade with
			// their ideas (foreign_keys is ON above); the source index
			// backs the cascade from a deleted imported feedback.
			let tx = conn.transaction()?;
			tx.execute_batch(REACTIONS_SCHEMA)?;
			tx.pragma_update(None, "user_version", 5)?;
			tx.commit()?;
		}
		// Self-healing backfill on every open: any row still carrying an
		// empty local_date (a crash mid-migration, an interrupted write)
		// is invisible to the streak and has_activity_today queries until
		// repaired. No-op once everything is backfilled. Imported ideas
		// keep their empty local_date on purpose (see NewIdea::imported).
		for table in ["ideas", "log_entries", "surveys"] {
			conn.execute_batch(&format!(
				"UPDATE {table} SET local_date = COALESCE(NULLIF(date(created_at, 'localtime'), ''), date('now', 'localtime')) WHERE local_date = ''{};",
				own_activity_filter(table)
			))?;
		}
		// The authoritative user timezone: read once at open so day
		// boundaries resolve without a settings query on every write.
		// Empty/missing/unparseable rows mean OS-local (the behavior
		// before the setting existed).
		let stored_zone: Option<String> = conn
			.query_row(
				"SELECT value FROM settings WHERE key = ?1",
				params![USER_TIMEZONE],
				|row| row.get(0),
			)
			.optional()?
			.flatten();
		let zone = stored_zone.as_deref().and_then(parse_zone);
		Ok(Self {
			conn: Mutex::new(conn),
			zone: Mutex::new(zone),
		})
	}

	/// The active zone: the parsed stored `user_timezone`, or None when
	/// unset, empty or unparseable (= OS-local). The reminder scheduler
	/// reads the same source so its wall clock and the daily boundaries
	/// can never disagree.
	pub fn active_zone(&self) -> Option<Tz> {
		*self.zone.lock().unwrap_or_else(|e| e.into_inner())
	}

	/// Keep the zone mirror in lockstep with a just-written
	/// `user_timezone` row. Called only after the write committed, so a
	/// failed or rolled-back save leaves the previous zone active.
	fn store_zone(&self, value: &str) {
		*self.zone.lock().unwrap_or_else(|e| e.into_inner()) = parse_zone(value);
	}

	/// Today's calendar date in the active zone (the stored user
	/// timezone, or the OS zone).
	fn today_local(&self) -> NaiveDate {
		today_in(self.active_zone())
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
		if key == USER_TIMEZONE {
			// after the write committed, never before
			self.store_zone(value);
		}
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
		})?;
		// Same-save hook for the authoritative timezone: refresh the
		// resolved zone only after the transaction committed, so a
		// failed save leaves the previous zone active.
		if let Some((_, value)) = kv.iter().find(|(key, _)| *key == USER_TIMEZONE) {
			self.store_zone(value);
		}
		Ok(())
	}

	/// Remove a setting row entirely (used when secrets move to the
	/// keychain or are cleared); Ok if the row doesn't exist.
	pub fn delete_setting(&self, key: &str) -> Result<(), String> {
		self.lock()
			.execute("DELETE FROM settings WHERE key = ?1", params![key])
			.map_err(|e| format!("failed to delete setting '{key}': {e}"))?;
		Ok(())
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

	const IDEA_SUMMARY_COLS: &'static str = "id, title, idea_type, result, CASE WHEN result = '' THEN transcript END AS transcript, parent_idea_id, is_unread, creator_name, creator_email, created_at";

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
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, ?12, ?13, ?14)",
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

	// ---- reactions ----

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
			let exists: i64 = conn
				.query_row(
					"SELECT EXISTS(SELECT 1 FROM ideas WHERE id = ?1)",
					params![idea_id],
					|row| row.get(0),
				)
				.map_err(|e| format!("failed to read idea {idea_id}: {e}"))?;
			if exists == 0 {
				return Err(format!("idea {idea_id} not found"));
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

	// ---- logs / surveys / daily ----

	pub fn insert_log(
		&self,
		id: &str,
		answers: &[crate::types::LogAnswerItem],
	) -> Result<(), String> {
		let json = serde_json::to_string(answers).unwrap_or_else(|_| "[]".into());
		let today = self.today_local().format("%Y-%m-%d").to_string();
		self.with_tx(|conn| {
			conn.execute(
				"INSERT INTO log_entries (id, answers, created_at, local_date) VALUES (?1, ?2, ?3, ?4)",
				params![id, json, now_iso(), today],
			)
			.map_err(|e| format!("failed to save daily log: {e}"))?;
			conn.execute(
				"INSERT INTO daily (date, log_id, is_completed, created_at) VALUES (?1, ?2, 0, ?3)
				 ON CONFLICT(date) DO UPDATE SET log_id = ?2",
				params![today, id, now_iso()],
			)
			.map_err(|e| format!("failed to save daily log: {e}"))?;
			Ok(())
		})
	}

	pub fn get_log_answers_today(&self) -> Option<Vec<crate::types::LogAnswerItem>> {
		let today = self.today_local().format("%Y-%m-%d").to_string();
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
		let today = self.today_local().format("%Y-%m-%d").to_string();
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
				"INSERT INTO daily (date, survey_id, created_at) VALUES (?1, ?2, ?3)
				 ON CONFLICT(date) DO UPDATE SET survey_id = ?2",
				params![today, id, now_iso()],
			)
			.map_err(|e| format!("failed to save survey: {e}"))?;
			Ok(())
		})
	}

	/// Mark today's daily row complete when `idea_id` is its intent.
	/// Ideas without a (today's) daily association match zero rows; the
	/// day/timezone semantics stay exactly as they were.
	fn complete_daily_tx(conn: &Connection, idea_id: &str, today: NaiveDate) -> Result<(), String> {
		conn.execute(
			"UPDATE daily SET is_completed = 1 WHERE date = ?1 AND intent_idea_id = ?2",
			params![today.format("%Y-%m-%d").to_string(), idea_id],
		)
		.map_err(|e| format!("failed to complete daily intent: {e}"))?;
		Ok(())
	}

	pub fn mark_daily_completed(&self, idea_id: &str) -> Result<(), String> {
		let today = self.today_local();
		self.with_tx(|conn| Self::complete_daily_tx(conn, idea_id, today))
	}

	/// True if any idea, log or survey was recorded today (local time) -
	/// used to skip the daily reminder on already-active days.
	pub fn has_activity_today(&self) -> bool {
		let today = self.today_local().format("%Y-%m-%d").to_string();
		let conn = self.lock();
		for table in ["ideas", "log_entries", "surveys"] {
			// Imports are filtered explicitly, not only through their empty
			// local_date: older builds let the open-time backfill stamp a
			// date onto them.
			let sql = format!(
				"SELECT EXISTS(SELECT 1 FROM {table} WHERE local_date = ?1{})",
				own_activity_filter(table)
			);
			if let Ok(1) = conn.query_row(&sql, params![today], |row| row.get::<_, i64>(0)) {
				return true;
			}
		}
		false
	}

	pub fn get_daily_status(&self) -> DailyStatus {
		let today = self.today_local().format("%Y-%m-%d").to_string();
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
			let sql = format!(
				"SELECT DISTINCT local_date FROM {table} WHERE local_date != ''{}",
				own_activity_filter(table)
			);
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
		let mut day = Some(self.today_local());
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

	fn temp_db_path() -> (std::path::PathBuf, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		(dir.path().join("brainstory-test.db"), dir)
	}

	#[test]
	fn survey_does_not_complete_the_daily_intent() {
		let (path, _dir) = temp_db_path();
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
	}

	#[test]
	fn update_idea_reports_missing_ideas() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		let err = db
			.update_idea("ghost", Some("t"), None, None, None)
			.expect_err("updating a missing idea must fail");
		assert!(err.contains("not found"), "unexpected error: {err}");
	}

	#[test]
	fn failed_daily_completion_rolls_back_the_finished_idea() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		let draft = vec![ChatMessage {
			role: "user".into(),
			content: "talked through the day".into(),
		}];
		db.create_daily_intent_idea("i1", "", "", &draft, &serde_json::json!({}))
			.unwrap();
		assert!(!db.get_daily_status().is_completed);
		// a failure injected into the daily-completion write only
		db.lock()
			.execute_batch(
				"CREATE TRIGGER fail_daily_complete BEFORE UPDATE ON daily
				 WHEN NEW.is_completed = 1
				 BEGIN SELECT RAISE(ABORT, 'injected completion failure'); END;",
			)
			.unwrap();
		// the save sequence: persist the finished idea, then complete the day
		let finished = vec![ChatMessage {
			role: "assistant".into(),
			content: "all done".into(),
		}];
		let outcome = db
			.update_idea(
				"i1",
				Some("Derived Title"),
				Some("## Finished"),
				Some(&finished),
				None,
			)
			.and_then(|()| db.mark_daily_completed("i1"));
		let err = outcome.expect_err("the injected completion failure must surface");
		assert!(
			err.contains("injected completion failure"),
			"unexpected error: {err}"
		);
		// all-or-nothing: the idea must still be the draft it was
		let idea = db.get_idea("i1").unwrap().unwrap();
		assert_eq!(
			idea.result.as_deref(),
			Some(""),
			"result committed even though the daily completion failed (partial commit)"
		);
		assert_eq!(idea.title, "", "title must roll back with the result");
		let transcript = idea.transcript.expect("draft transcript kept");
		assert_eq!(
			transcript.len(),
			1,
			"transcript must roll back with the result"
		);
		assert_eq!(transcript[0].content, "talked through the day");
		assert!(
			!db.get_daily_status().is_completed,
			"the day must not be completed either"
		);
	}

	#[test]
	fn finishing_the_intent_completes_the_day_in_the_same_transaction() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		db.create_daily_intent_idea("i1", "", "", &[], &serde_json::json!({}))
			.unwrap();
		assert!(!db.get_daily_status().is_completed);
		db.update_idea("i1", Some("Derived Title"), Some("## Result"), None, None)
			.unwrap();
		let idea = db.get_idea("i1").unwrap().unwrap();
		assert_eq!(idea.result.as_deref(), Some("## Result"));
		assert_eq!(idea.title, "Derived Title");
		assert!(
			db.get_daily_status().is_completed,
			"a nonempty result completes the day together with the idea update"
		);
	}

	#[test]
	fn updating_an_ordinary_idea_without_a_daily_association_still_saves() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		db.insert_idea(NewIdea {
			id: "note",
			title: "",
			idea_type: IdeaType::Original,
			result: "",
			metadata: &serde_json::json!({}),
			..Default::default()
		})
		.unwrap();
		db.update_idea(
			"note",
			Some("Note"),
			Some("a plain finished idea"),
			None,
			None,
		)
		.unwrap();
		let idea = db.get_idea("note").unwrap().unwrap();
		assert_eq!(idea.result.as_deref(), Some("a plain finished idea"));
		assert_eq!(idea.title, "Note");
		// no daily row is created or completed by an ordinary idea
		let daily_rows: i64 = db
			.lock()
			.query_row("SELECT COUNT(*) FROM daily", [], |r| r.get(0))
			.unwrap();
		assert_eq!(daily_rows, 0, "ordinary ideas never touch the daily table");
		assert!(!db.get_daily_status().is_completed);
	}

	#[test]
	fn empty_or_absent_result_leaves_daily_completion_unchanged() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		db.create_daily_intent_idea("i1", "", "", &[], &serde_json::json!({}))
			.unwrap();
		// an explicitly empty result saves but does not complete the day
		db.update_idea("i1", None, Some(""), None, None).unwrap();
		assert_eq!(
			db.get_idea("i1").unwrap().unwrap().result.as_deref(),
			Some("")
		);
		assert!(!db.get_daily_status().is_completed);
		// a transcript-only autosave (absent result) doesn't complete either
		let chat = vec![ChatMessage {
			role: "user".into(),
			content: "more".into(),
		}];
		db.update_idea("i1", None, None, Some(&chat), None).unwrap();
		assert!(!db.get_daily_status().is_completed);
		// once the day is complete, later saves must not undo it
		db.update_idea("i1", None, Some("## Done"), None, None)
			.unwrap();
		assert!(db.get_daily_status().is_completed);
		db.update_idea("i1", None, None, Some(&chat), None).unwrap();
		assert!(db.get_daily_status().is_completed);
	}

	#[test]
	fn repeating_the_finished_intent_update_is_idempotent() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		db.create_daily_intent_idea("i1", "", "", &[], &serde_json::json!({}))
			.unwrap();
		db.update_idea("i1", Some("T"), Some("## First"), None, None)
			.unwrap();
		assert!(db.get_daily_status().is_completed);
		db.update_idea("i1", Some("T"), Some("## Second"), None, None)
			.unwrap();
		assert!(
			db.get_daily_status().is_completed,
			"a repeated finish keeps the day completed"
		);
		assert_eq!(
			db.get_idea("i1").unwrap().unwrap().result.as_deref(),
			Some("## Second")
		);
	}

	#[test]
	fn update_after_the_intent_idea_was_deleted_reports_not_found() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		db.create_daily_intent_idea("i1", "", "", &[], &serde_json::json!({}))
			.unwrap();
		assert!(db.delete_idea("i1").unwrap());
		let err = db
			.update_idea("i1", None, Some("## Result"), None, None)
			.expect_err("updating a deleted idea must fail");
		assert!(err.contains("not found"), "unexpected error: {err}");
		// the day row survives the delete untouched and uncompleted
		let status = db.get_daily_status();
		assert!(!status.is_completed);
		assert_eq!(status.intent_idea_id, None);
	}

	#[test]
	fn imported_ideas_do_not_count_toward_activity() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		let yesterday = (Utc::now().naive_utc() - chrono::Duration::days(1))
			.format("%Y-%m-%dT%H:%M:%S")
			.to_string();
		// an imported idea dated yesterday must not feed the streak: the
		// activity happened on the author's machine, not here
		db.insert_idea(NewIdea::imported(NewIdea {
			id: "imp",
			title: "Imported",
			idea_type: IdeaType::Original,
			result: "r",
			metadata: &serde_json::json!({ "imported": true }),
			creator_name: Some("Ada"),
			created_at: Some(&yesterday),
			..Default::default()
		}))
		.expect("insert imported");
		assert_eq!(db.get_daily_status().streak, 0, "import must not count");
		assert!(!db.has_activity_today());
		// a local idea dated today still counts normally
		db.insert_idea(NewIdea {
			id: "mine",
			title: "Mine",
			idea_type: IdeaType::Original,
			result: "r",
			metadata: &serde_json::json!({}),
			..Default::default()
		})
		.expect("insert local");
		assert!(db.has_activity_today());
	}

	#[test]
	fn imported_ideas_still_do_not_count_after_reopening() {
		let (path, _dir) = temp_db_path();
		let now = Utc::now()
			.naive_utc()
			.format("%Y-%m-%dT%H:%M:%S")
			.to_string();
		let yesterday = (Utc::now().naive_utc() - chrono::Duration::days(1))
			.format("%Y-%m-%dT%H:%M:%S")
			.to_string();
		let meta = serde_json::json!({ "imported": true });
		{
			let db = Db::open(&path).expect("open");
			db.insert_idea(NewIdea::imported(NewIdea {
				id: "imp-today",
				title: "Imported today",
				idea_type: IdeaType::Original,
				result: "r",
				metadata: &meta,
				created_at: Some(&now),
				..Default::default()
			}))
			.expect("insert imported idea");
			db.insert_idea(NewIdea::imported(NewIdea {
				id: "imp-feedback",
				title: "Imported feedback",
				idea_type: IdeaType::Feedback,
				result: "r",
				metadata: &meta,
				parent_idea_id: Some("imp-today"),
				created_at: Some(&yesterday),
				..Default::default()
			}))
			.expect("insert imported feedback");
			assert_eq!(db.get_daily_status().streak, 0);
			assert!(!db.has_activity_today());
		}
		// the every-open backfill must not hand the rows a local date...
		let db = Db::open(&path).expect("reopen");
		assert_eq!(
			db.get_daily_status().streak,
			0,
			"import counted after reopen"
		);
		assert!(!db.has_activity_today(), "import counted after reopen");
		{
			let conn = db.lock();
			let dated: i64 = conn
				.query_row(
					"SELECT COUNT(*) FROM ideas WHERE local_date != ''",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(dated, 0, "imported rows keep their empty local_date");
			// ...and rows an older build already backfilled still don't count
			conn.execute("UPDATE ideas SET local_date = date('now', 'localtime')", [])
				.unwrap();
		}
		assert_eq!(
			db.get_daily_status().streak,
			0,
			"legacy-backfilled import counted"
		);
		assert!(!db.has_activity_today(), "legacy-backfilled import counted");
	}

	#[test]
	fn daily_rows_record_when_they_were_created() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		let created_at = |db: &Db| -> String {
			db.lock()
				.query_row("SELECT created_at FROM daily", [], |r| r.get(0))
				.unwrap()
		};
		db.insert_log("l1", &[]).unwrap();
		let first = created_at(&db);
		assert!(
			chrono::NaiveDateTime::parse_from_str(&first, "%Y-%m-%dT%H:%M:%S").is_ok(),
			"naive-UTC timestamp like every other created_at, got {first:?}"
		);
		// later activity on the same day updates the row, not its birth
		db.lock()
			.execute("UPDATE daily SET created_at = '2000-01-01T00:00:00'", [])
			.unwrap();
		db.insert_survey("s1", None, &serde_json::json!({}))
			.unwrap();
		db.create_daily_intent_idea("i1", "T", "", &[], &serde_json::json!({}))
			.unwrap();
		assert_eq!(created_at(&db), "2000-01-01T00:00:00");

		// every statement that can create the row stamps it
		for create in [
			|db: &Db| db.insert_survey("s2", None, &serde_json::json!({})),
			|db: &Db| db.create_daily_intent_idea("i2", "T", "", &[], &serde_json::json!({})),
		] {
			db.lock().execute("DELETE FROM daily", []).unwrap();
			create(&db).unwrap();
			assert_ne!(created_at(&db), "", "row created without a timestamp");
		}
	}

	#[test]
	fn new_intent_draft_resets_completion() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		// the day is completed by a finished intent...
		db.create_daily_intent_idea("done", "T", "## Result", &[], &serde_json::json!({}))
			.unwrap();
		assert!(db.get_daily_status().is_completed);
		// ...so a new empty draft for the same day must not inherit it
		db.create_daily_intent_idea("draft", "T", "", &[], &serde_json::json!({}))
			.unwrap();
		let status = db.get_daily_status();
		assert!(
			!status.is_completed,
			"a new draft intent resets the completed flag"
		);
		assert_eq!(status.intent_idea_id.as_deref(), Some("draft"));
	}

	#[test]
	fn generating_the_intent_result_completes_the_day() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		// draft intent: recorded but not completed
		db.create_daily_intent_idea("i1", "T", "", &[], &serde_json::json!({}))
			.unwrap();
		assert!(!db.get_daily_status().is_completed);
		// an intent created with its result already present completes the day
		db.create_daily_intent_idea("i2", "T", "## Result", &[], &serde_json::json!({}))
			.unwrap();
		assert!(db.get_daily_status().is_completed);
	}

	#[test]
	fn open_rejects_newer_schema_version() {
		let (path, _dir) = temp_db_path();
		{
			let conn = Connection::open(&path).unwrap();
			conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
				.unwrap();
		}
		let err = match Db::open(&path) {
			Ok(_) => panic!("a newer schema must be refused"),
			Err(e) => e,
		};
		assert!(
			matches!(err, super::OpenError::NewerSchema { found, .. } if found == SCHEMA_VERSION + 1),
			"unexpected error: {err:?}"
		);
		// the file must be left exactly as found: no stamping, no migration
		let conn = Connection::open(&path).unwrap();
		let version: i64 = conn
			.query_row("PRAGMA user_version", [], |r| r.get(0))
			.unwrap();
		assert_eq!(version, SCHEMA_VERSION + 1);
	}

	#[test]
	fn migration_backfill_is_idempotent_after_partial_run() {
		let (path, _dir) = temp_db_path();
		{
			// a crash after the ALTER TABLE but before the backfill (and
			// before the user_version bump): the column exists, rows keep
			// local_date = '', user_version still says 2
			let conn = Connection::open(&path).unwrap();
			conn.execute_batch(BASELINE_SCHEMA).unwrap();
			conn.execute_batch("ALTER TABLE daily ADD COLUMN created_at TEXT NOT NULL DEFAULT '';")
				.unwrap();
			for table in ["ideas", "log_entries", "surveys"] {
				conn.execute_batch(&format!(
					"ALTER TABLE {table} ADD COLUMN local_date TEXT NOT NULL DEFAULT '';"
				))
				.unwrap();
			}
			conn.execute_batch(
				"INSERT INTO ideas (id, created_at) VALUES ('old', strftime('%Y-%m-%dT%H:%M:%S', 'now', '-1 day'));",
			)
			.unwrap();
			conn.pragma_update(None, "user_version", 2).unwrap();
		}
		let db = Db::open(&path).expect("finish the interrupted migration");
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
		}
		// the row counts toward the streak again, not just after the
		// healing open but on every later open too
		assert!(db.get_daily_status().streak >= 1);
		drop(db);
		let db = Db::open(&path).expect("reopen");
		assert!(db.get_daily_status().streak >= 1, "backfill stays stable");
	}

	#[test]
	fn migrates_v2_database_and_backfills_local_date() {
		let (path, _dir) = temp_db_path();
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
	}

	#[test]
	fn delete_idea_removes_the_whole_feedback_subtree() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		let meta = serde_json::json!({});
		let empty: Vec<ChatMessage> = vec![];
		db.insert_idea(NewIdea {
			id: "parent",
			title: "P",
			idea_type: IdeaType::Original,
			result: "r",
			transcript: &empty,
			metadata: &meta,
			..Default::default()
		})
		.unwrap();
		db.insert_idea(NewIdea {
			id: "child",
			title: "C",
			idea_type: IdeaType::Feedback,
			result: "r",
			transcript: &empty,
			metadata: &meta,
			parent_idea_id: Some("parent"),
			..Default::default()
		})
		.unwrap();
		// a child of the child: the old delete-children-only logic left
		// this row orphaned in the library
		db.insert_idea(NewIdea {
			id: "grandchild",
			title: "G",
			idea_type: IdeaType::Feedback,
			result: "r",
			transcript: &empty,
			metadata: &meta,
			parent_idea_id: Some("child"),
			..Default::default()
		})
		.unwrap();
		let deleted = db.delete_idea("parent").unwrap();
		assert!(deleted);
		assert!(db.get_idea("parent").unwrap().is_none());
		assert!(db.get_idea("child").unwrap().is_none());
		assert!(db.get_idea("grandchild").unwrap().is_none());
		assert!(db.list_ideas().unwrap().is_empty());
	}

	#[test]
	fn insert_idea_rejects_a_missing_parent() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		let meta = serde_json::json!({});
		let err = db
			.insert_idea(NewIdea {
				id: "kid",
				title: "K",
				idea_type: IdeaType::Feedback,
				result: "r",
				metadata: &meta,
				parent_idea_id: Some("ghost"),
				..Default::default()
			})
			.expect_err("missing parent must fail");
		assert!(err.contains("not found"), "unexpected error: {err}");
		assert!(db.get_idea("kid").unwrap().is_none(), "nothing inserted");
	}

	#[test]
	fn foreign_keys_are_enforced_at_the_schema_level() {
		let (path, _dir) = temp_db_path();
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
			conn.execute(
				"INSERT INTO ideas (id, created_at) VALUES ('p', '2026-01-01T00:00:00')",
				[],
			)
			.unwrap();
			conn.execute(
				"INSERT INTO ideas (id, created_at, parent_idea_id) VALUES ('c', '2026-01-01T00:00:00', 'p')",
				[],
			)
			.unwrap();
			conn.execute("DELETE FROM ideas WHERE id = 'p'", [])
				.unwrap();
			let orphans: i64 = conn
				.query_row("SELECT COUNT(*) FROM ideas WHERE id = 'c'", [], |row| {
					row.get(0)
				})
				.unwrap();
			assert_eq!(orphans, 0, "cascade must remove the child");
		}
	}

	#[test]
	fn get_idea_title_is_a_lightweight_read() {
		let (path, _dir) = temp_db_path();
		let db = Db::open(&path).expect("open");
		let meta = serde_json::json!({});
		db.insert_idea(NewIdea {
			id: "t1",
			title: "The Title",
			idea_type: IdeaType::Original,
			result: "r",
			metadata: &meta,
			..Default::default()
		})
		.unwrap();
		assert_eq!(
			db.get_idea_title("t1").unwrap().as_deref(),
			Some("The Title")
		);
		assert_eq!(db.get_idea_title("missing").unwrap(), None);
	}
}

#[cfg(test)]
mod coverage_tests {
	use super::*;
	use serde_json::json;

	/// A fresh database in its own temp dir, removed (with its WAL
	/// sidecars) when the returned TempDir drops.
	fn db() -> (Db, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		(Db::open(&dir.path().join("cov.db")).expect("open"), dir)
	}

	fn idea(db: &Db, id: &str, created_at: &str) {
		db.insert_idea(NewIdea {
			id,
			title: id,
			idea_type: IdeaType::Original,
			result: "r",
			metadata: &json!({}),
			created_at: Some(created_at),
			..Default::default()
		})
		.expect("insert");
	}

	#[test]
	fn streak_counts_consecutive_days_and_stops_at_gap() {
		let (db, _dir) = db();
		let ts = |offset: i64| {
			(db.today_local() - chrono::Duration::days(offset))
				.and_hms_opt(12, 0, 0)
				.unwrap()
				.format("%Y-%m-%dT%H:%M:%S")
				.to_string()
		};
		// today, yesterday, 2 days ago; a hole on day 3; activity on day 4
		idea(&db, "t", &ts(0));
		idea(&db, "y", &ts(1));
		idea(&db, "d2", &ts(2));
		idea(&db, "d4", &ts(4));
		assert_eq!(db.get_daily_status().streak, 3, "stops at the gap");
	}

	#[test]
	fn list_ideas_groups_children_under_parents_newest_first() {
		let (db, _dir) = db();
		idea(&db, "old", "2026-01-01T00:00:00");
		idea(&db, "new", "2026-02-01T00:00:00");
		db.insert_idea(NewIdea {
			id: "c1",
			title: "C1",
			idea_type: IdeaType::Feedback,
			result: "r",
			metadata: &json!({}),
			parent_idea_id: Some("old"),
			created_at: Some("2026-01-02T00:00:00"),
			..Default::default()
		})
		.unwrap();
		db.insert_idea(NewIdea {
			id: "c2",
			title: "C2",
			idea_type: IdeaType::Feedback,
			result: "r",
			metadata: &json!({}),
			parent_idea_id: Some("old"),
			created_at: Some("2026-01-03T00:00:00"),
			..Default::default()
		})
		.unwrap();
		let ideas = db.list_ideas().unwrap();
		// top level: newest first, children grouped (not returned as roots)
		assert_eq!(ideas.len(), 2);
		assert_eq!(ideas[0].id, "new");
		assert_eq!(ideas[1].id, "old");
		let feedback = ideas[1].feedback.as_ref().unwrap();
		assert_eq!(feedback.len(), 2);
		assert_eq!(feedback[0].id, "c2", "children newest first");
	}

	#[test]
	fn list_ideas_skips_what_the_library_grid_never_reads() {
		let (db, _dir) = db();
		let chat = vec![ChatMessage {
			role: "user".into(),
			content: "a long brainstorm".into(),
		}];
		db.insert_idea(NewIdea {
			id: "done",
			title: "Done",
			idea_type: IdeaType::Original,
			result: "## Result\nbody",
			structured_result: Some(&json!({ "feedback_items": [] })),
			transcript: &chat,
			metadata: &json!({}),
			..Default::default()
		})
		.unwrap();
		db.insert_idea(NewIdea {
			id: "draft",
			title: "",
			idea_type: IdeaType::Original,
			result: "",
			transcript: &chat,
			metadata: &json!({}),
			..Default::default()
		})
		.unwrap();
		let ideas = db.list_ideas().unwrap();
		let done = ideas.iter().find(|i| i.id == "done").unwrap();
		let draft = ideas.iter().find(|i| i.id == "draft").unwrap();
		// finished ideas: the grid shows title + result preview only
		assert!(
			done.transcript.is_none(),
			"no transcript for finished ideas"
		);
		assert!(done.result_json.is_none() && done.structured_result.is_none());
		assert_eq!(done.result.as_deref(), Some("## Result\nbody"));
		// drafts: the grid previews the last user message of the transcript
		assert_eq!(
			draft.transcript.as_ref().map(|t| t[0].content.as_str()),
			Some("a long brainstorm")
		);
		// the full read still has everything
		let full = db.get_idea("done").unwrap().unwrap();
		assert!(full.transcript.is_some() && full.result_json.is_some());
	}

	#[test]
	fn delete_idea_clears_daily_intent_and_survey_links() {
		let (db, _dir) = db();
		// the production path that links an idea as today's intent
		db.create_daily_intent_idea("target", "target", "r", &[], &json!({}))
			.unwrap();
		db.insert_survey("s1", Some("target"), &json!({})).unwrap();
		let today = db.today_local().format("%Y-%m-%d").to_string();
		{
			let conn = db.lock();
			let linked: i64 = conn
				.query_row(
					"SELECT (SELECT COUNT(*) FROM daily WHERE intent_idea_id = 'target')
					 + (SELECT COUNT(*) FROM surveys WHERE idea_id = 'target')",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(linked, 2, "links exist before delete");
			let _ = today;
		}
		db.delete_idea("target").unwrap();
		{
			let conn = db.lock();
			let linked: i64 = conn
				.query_row(
					"SELECT (SELECT COUNT(*) FROM daily WHERE intent_idea_id = 'target')
					 + (SELECT COUNT(*) FROM surveys WHERE idea_id = 'target')",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(linked, 0, "no dangling references after delete");
			let today_row: Option<String> = conn
				.query_row(
					"SELECT date FROM daily WHERE date = ?1",
					params![today],
					|r| r.get(0),
				)
				.optional()
				.unwrap();
			assert!(today_row.is_some(), "the daily row itself survives");
		}
	}

	#[test]
	fn delete_idea_clears_links_for_every_deleted_descendant() {
		let (db, _dir) = db();
		idea(&db, "parent", "2026-01-01T00:00:00");
		for (id, parent) in [("child", Some("parent")), ("grandchild", Some("child"))] {
			db.insert_idea(NewIdea {
				id,
				title: id,
				idea_type: IdeaType::Feedback,
				result: "r",
				metadata: &json!({}),
				parent_idea_id: parent,
				created_at: Some("2026-01-02T00:00:00"),
				..Default::default()
			})
			.unwrap();
		}
		idea(&db, "other", "2026-01-01T00:00:00");
		// every subtree member is linked to a survey, a past day's
		// intent points at the grandchild (no production path makes a
		// descendant an intent, but nothing in the schema forbids it),
		// and an unrelated idea keeps its own survey link
		db.insert_survey("s-parent", Some("parent"), &json!({}))
			.unwrap();
		db.insert_survey("s-child", Some("child"), &json!({}))
			.unwrap();
		db.insert_survey("s-grandchild", Some("grandchild"), &json!({}))
			.unwrap();
		db.insert_survey("s-other", Some("other"), &json!({}))
			.unwrap();
		db.lock()
			.execute(
				"INSERT INTO daily (date, intent_idea_id, is_completed, created_at)
				 VALUES ('2026-01-02', 'grandchild', 1, '2026-01-02T00:00:00')",
				[],
			)
			.unwrap();

		assert!(db.delete_idea("parent").unwrap());

		{
			let conn = db.lock();
			for id in ["parent", "child", "grandchild"] {
				let linked: i64 = conn
					.query_row(
						"SELECT COUNT(*) FROM surveys WHERE idea_id = ?1",
						params![id],
						|r| r.get(0),
					)
					.unwrap();
				assert_eq!(linked, 0, "survey still references deleted idea {id}");
			}
			let survived: i64 = conn
				.query_row(
					"SELECT COUNT(*) FROM surveys WHERE id IN ('s-parent', 's-child', 's-grandchild') AND idea_id IS NULL",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(
				survived, 3,
				"survey rows must survive with their link cleared, not be deleted"
			);
			let unrelated: i64 = conn
				.query_row(
					"SELECT COUNT(*) FROM surveys WHERE id = 's-other' AND idea_id = 'other'",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(unrelated, 1, "the unrelated idea's survey link survives");
			let intents: i64 = conn
				.query_row(
					"SELECT COUNT(*) FROM daily WHERE intent_idea_id IS NOT NULL",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(intents, 0, "a daily intent still names a deleted idea");
			let day_kept: i64 = conn
				.query_row(
					"SELECT COUNT(*) FROM daily WHERE date = '2026-01-02' AND intent_idea_id IS NULL AND is_completed = 1",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(day_kept, 1, "the day row itself survives");
			let dangling: i64 = conn
				.query_row(
					"SELECT COUNT(*) FROM surveys WHERE idea_id IS NOT NULL AND idea_id NOT IN (SELECT id FROM ideas)",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(dangling, 0, "no dangling survey reference remains");
		}
		assert!(db.get_idea("other").unwrap().is_some());
	}

	#[test]
	fn delete_idea_is_clean_for_repeats_and_missing_ids() {
		let (db, _dir) = db();
		idea(&db, "solo", "2026-01-01T00:00:00");
		db.insert_survey("s1", Some("solo"), &json!({})).unwrap();
		assert!(db.delete_idea("solo").unwrap());
		// deleting again, and deleting an id that never existed, are
		// calm no-ops rather than errors
		assert!(!db.delete_idea("solo").unwrap());
		assert!(!db.delete_idea("ghost").unwrap());
		assert!(db.list_ideas().unwrap().is_empty());
		let linked: i64 = db
			.lock()
			.query_row(
				"SELECT COUNT(*) FROM surveys WHERE idea_id IS NOT NULL",
				[],
				|r| r.get(0),
			)
			.unwrap();
		assert_eq!(linked, 0);
	}

	#[test]
	fn failed_delete_idea_rolls_back_rows_and_links() {
		let (db, _dir) = db();
		idea(&db, "parent", "2026-01-01T00:00:00");
		db.insert_idea(NewIdea {
			id: "child",
			title: "child",
			idea_type: IdeaType::Feedback,
			result: "r",
			metadata: &json!({}),
			parent_idea_id: Some("parent"),
			created_at: Some("2026-01-02T00:00:00"),
			..Default::default()
		})
		.unwrap();
		db.insert_survey("s-child", Some("child"), &json!({}))
			.unwrap();
		db.lock()
			.execute(
				"INSERT INTO daily (date, intent_idea_id, is_completed, created_at)
				 VALUES ('2026-01-02', 'child', 0, '2026-01-02T00:00:00')",
				[],
			)
			.unwrap();
		db.lock()
			.execute_batch(
				"CREATE TRIGGER fail_delete BEFORE DELETE ON ideas
				 WHEN OLD.id = 'child'
				 BEGIN SELECT RAISE(ABORT, 'injected delete failure'); END;",
			)
			.unwrap();
		// the injected failure must roll back the whole transaction:
		// rows AND their survey/intent links stay exactly as they were
		let err = db.delete_idea("parent").unwrap_err();
		assert!(
			err.contains("injected delete failure"),
			"unexpected error: {err}"
		);
		{
			let conn = db.lock();
			for id in ["parent", "child"] {
				let kept: i64 = conn
					.query_row(
						"SELECT COUNT(*) FROM ideas WHERE id = ?1",
						params![id],
						|r| r.get(0),
					)
					.unwrap();
				assert_eq!(kept, 1, "{id} must survive the rollback");
			}
			let survey_link: Option<String> = conn
				.query_row(
					"SELECT idea_id FROM surveys WHERE id = 's-child'",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(
				survey_link.as_deref(),
				Some("child"),
				"survey link must survive the rollback"
			);
			let intent_link: Option<String> = conn
				.query_row(
					"SELECT intent_idea_id FROM daily WHERE date = '2026-01-02'",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(
				intent_link.as_deref(),
				Some("child"),
				"daily intent must survive the rollback"
			);
		}
		// with the failure gone the same delete succeeds and clears
		db.lock().execute_batch("DROP TRIGGER fail_delete").unwrap();
		assert!(db.delete_idea("parent").unwrap());
		{
			let conn = db.lock();
			let ideas: i64 = conn
				.query_row("SELECT COUNT(*) FROM ideas", [], |r| r.get(0))
				.unwrap();
			assert_eq!(ideas, 0);
			let survey_link: Option<String> = conn
				.query_row(
					"SELECT idea_id FROM surveys WHERE id = 's-child'",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(survey_link, None, "survey link cleared on success");
			let intent_link: Option<String> = conn
				.query_row(
					"SELECT intent_idea_id FROM daily WHERE date = '2026-01-02'",
					[],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(intent_link, None, "daily intent cleared on success");
		}
	}

	#[test]
	fn result_to_json_keeps_title_slot_and_splits_h1_h2() {
		let parsed = Db::result_to_json("# Title\n\n## One\nfirst\n\n## Two\nsecond");
		let sections = parsed.as_array().unwrap();
		// a document that starts with a heading still gets the empty
		// title slot at index 0, so 1-based heading ordinals from the
		// feedback JSON line up with array indices
		assert_eq!(sections.len(), 4, "{parsed}");
		assert_eq!(sections[0]["heading"], "", "index 0 is the title slot");
		assert_eq!(sections[1]["heading"], "# Title");
		assert_eq!(sections[2]["heading"], "## One");
		assert_eq!(sections[2]["body"], "first");
		assert_eq!(sections[3]["heading"], "## Two");
		// body text before any heading fills the title slot itself
		let led = Db::result_to_json("intro line\n\n## Only\nbody");
		let arr = led.as_array().unwrap();
		assert_eq!(arr.len(), 2);
		assert_eq!(arr[0]["body"], "intro line");
		assert_eq!(arr[1]["heading"], "## Only");
	}
}

#[cfg(test)]
mod timezone_tests {
	use super::*;
	use crate::keys::setting::USER_TIMEZONE;

	fn insert_dated_idea(db: &Db, id: &str, created_at: &str) {
		db.insert_idea(NewIdea {
			id,
			title: id,
			idea_type: IdeaType::Original,
			result: "r",
			metadata: &serde_json::json!({}),
			created_at: Some(created_at),
			..Default::default()
		})
		.expect("insert");
	}

	fn local_date_of(db: &Db, id: &str) -> String {
		db.lock()
			.query_row(
				"SELECT local_date FROM ideas WHERE id = ?1",
				params![id],
				|r| r.get(0),
			)
			.unwrap()
	}

	#[test]
	fn day_boundaries_follow_the_stored_zone_not_the_os_zone() {
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join("tz.db")).expect("open");
		db.set_settings(&[(USER_TIMEZONE, "Asia/Tokyo".to_string())])
			.expect("store zone");
		assert_eq!(
			db.active_zone().map(|tz| tz.name().to_string()),
			Some("Asia/Tokyo".to_string())
		);
		// 20:30 UTC on July 10 is already July 11 at UTC+9; the frozen
		// activity day must follow the stored zone
		insert_dated_idea(&db, "i", "2026-07-10T20:30:00");
		assert_eq!(
			local_date_of(&db, "i"),
			"2026-07-11",
			"the stored timezone must drive the day boundary"
		);
	}

	#[test]
	fn the_stored_zone_is_resolved_again_at_open() {
		let dir = tempfile::tempdir().expect("tempdir");
		let path = dir.path().join("tz-reopen.db");
		{
			let db = Db::open(&path).expect("open");
			db.set_settings(&[(USER_TIMEZONE, "Asia/Tokyo".to_string())])
				.expect("store zone");
		}
		let db = Db::open(&path).expect("reopen");
		assert_eq!(
			db.active_zone().map(|tz| tz.name().to_string()),
			Some("Asia/Tokyo".to_string()),
			"the zone is read from the stored row on every open"
		);
		insert_dated_idea(&db, "i", "2026-07-10T20:30:00");
		assert_eq!(
			local_date_of(&db, "i"),
			"2026-07-11",
			"a reopened database keeps freezing days in the stored zone"
		);
	}

	#[test]
	fn changing_the_zone_affects_only_future_writes() {
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join("tz-switch.db")).expect("open");
		db.set_settings(&[(USER_TIMEZONE, "Asia/Tokyo".to_string())])
			.expect("Tokyo");
		insert_dated_idea(&db, "tokyo", "2026-07-10T20:30:00");
		// a later save switches the active zone without a restart...
		db.set_settings(&[(USER_TIMEZONE, "America/New_York".to_string())])
			.expect("New York");
		assert_eq!(
			db.active_zone().map(|tz| tz.name().to_string()),
			Some("America/New_York".to_string())
		);
		// ...and the same UTC instant now freezes to the new zone's day
		insert_dated_idea(&db, "ny", "2026-07-10T20:30:00");
		assert_eq!(
			local_date_of(&db, "ny"),
			"2026-07-10",
			"20:30 UTC is still July 10 at UTC-4"
		);
		// ...while already-frozen history is NOT recomputed
		assert_eq!(
			local_date_of(&db, "tokyo"),
			"2026-07-11",
			"a zone switch must never rewrite frozen local dates"
		);
	}

	#[test]
	fn empty_or_unparseable_stored_zones_fall_back_to_os_local() {
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join("tz-fallback.db")).expect("open");
		assert_eq!(db.active_zone(), None, "no stored row = OS-local");
		// the equivalent of a hand-edited row naming an unknown zone
		db.set_settings(&[(USER_TIMEZONE, "Mars/Olympus_Mons".to_string())])
			.expect("store junk zone");
		assert_eq!(
			db.active_zone(),
			None,
			"an unparseable stored value falls back to OS-local, not an error"
		);
		db.set_settings(&[(USER_TIMEZONE, String::new())])
			.expect("store empty zone");
		assert_eq!(db.active_zone(), None, "an empty stored value = OS-local");
	}

	#[test]
	fn a_failed_settings_save_leaves_the_active_zone_unchanged() {
		let dir = tempfile::tempdir().expect("tempdir");
		let path = dir.path().join("tz-rollback.db");
		let db = Db::open(&path).expect("open");
		db.set_settings(&[(USER_TIMEZONE, "Asia/Tokyo".to_string())])
			.expect("Tokyo");
		{
			let conn = Connection::open(&path).unwrap();
			conn.execute_batch(
				"CREATE TRIGGER fail_zone BEFORE INSERT ON settings
				 WHEN NEW.key = 'user_timezone'
				 BEGIN SELECT RAISE(ABORT, 'disk on fire'); END;",
			)
			.unwrap();
		}
		let err = db
			.set_settings(&[(USER_TIMEZONE, "America/New_York".to_string())])
			.expect_err("the failing write surfaces");
		assert!(err.contains("disk on fire"), "unexpected: {err}");
		assert_eq!(
			db.active_zone().map(|tz| tz.name().to_string()),
			Some("Asia/Tokyo".to_string()),
			"a rolled-back save must not switch the active zone"
		);
	}
}

#[cfg(test)]
mod v4_tests {
	use super::*;

	#[test]
	fn every_open_backfills_rows_left_undated_at_v3_and_v4() {
		// The versioned migrations never run again on a v3/v4 file, so
		// rows written with an empty local_date (an interrupted write, a
		// build that missed the column) are only repaired by the
		// every-open backfill.
		for version in [3, 4] {
			let dir = tempfile::tempdir().expect("tempdir");
			let path = dir.path().join("t.db");
			{
				let conn = Connection::open(&path).unwrap();
				conn.execute_batch(BASELINE_SCHEMA).unwrap();
				conn.execute_batch(
					"ALTER TABLE daily ADD COLUMN created_at TEXT NOT NULL DEFAULT '';",
				)
				.unwrap();
				for table in ["ideas", "log_entries", "surveys"] {
					conn.execute_batch(&format!(
						"ALTER TABLE {table} ADD COLUMN local_date TEXT NOT NULL DEFAULT '';"
					))
					.unwrap();
				}
				conn.execute_batch(
					"INSERT INTO ideas (id, created_at) VALUES ('i', strftime('%Y-%m-%dT%H:%M:%S', 'now', '-1 day'));
					 INSERT INTO log_entries (id, answers, created_at) VALUES ('l', '[]', strftime('%Y-%m-%dT%H:%M:%S', 'now'));
					 INSERT INTO surveys (id, answers, created_at) VALUES ('s', '{}', strftime('%Y-%m-%dT%H:%M:%S', 'now'));",
				)
				.unwrap();
				conn.pragma_update(None, "user_version", version).unwrap();
			}
			let db = Db::open(&path).expect("open");
			{
				let conn = db.lock();
				for table in ["ideas", "log_entries", "surveys"] {
					let undated: i64 = conn
						.query_row(
							&format!("SELECT COUNT(*) FROM {table} WHERE local_date = ''"),
							[],
							|r| r.get(0),
						)
						.unwrap();
					assert_eq!(undated, 0, "v{version}: {table} row left undated");
				}
			}
			assert!(db.has_activity_today(), "v{version}: today's log counts");
			assert_eq!(
				db.get_daily_status().streak,
				2,
				"v{version}: yesterday's idea + today's log"
			);
		}
	}

	#[test]
	fn migration_v4_adds_local_date_indexes() {
		let dir = tempfile::tempdir().expect("tempdir");
		let path = dir.path().join("t.db");
		// a database exactly as schema version 3 left it (columns and
		// indexes present, local_date columns backfilled)
		{
			let conn = Connection::open(&path).unwrap();
			conn.execute_batch(BASELINE_SCHEMA).unwrap();
			conn.execute_batch("ALTER TABLE daily ADD COLUMN created_at TEXT NOT NULL DEFAULT '';")
				.unwrap();
			for table in ["ideas", "log_entries", "surveys"] {
				conn.execute_batch(&format!(
					"ALTER TABLE {table} ADD COLUMN local_date TEXT NOT NULL DEFAULT '';"
				))
				.unwrap();
			}
			conn.pragma_update(None, "user_version", 3).unwrap();
		}
		let db = Db::open(&path).expect("migrate v3 -> v4");
		{
			let conn = db.lock();
			let version: i64 = conn
				.query_row("PRAGMA user_version", [], |r| r.get(0))
				.unwrap();
			assert_eq!(version, SCHEMA_VERSION);
			for table in ["ideas", "log_entries", "surveys"] {
				let has: i64 = conn
					.query_row(
						&format!(
							"SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_{table}_local_date'"
						),
						[],
						|r| r.get(0),
					)
					.unwrap();
				assert_eq!(has, 1, "{table} local_date index exists");
			}
		}
	}
}

#[cfg(test)]
mod reaction_tests {
	use super::*;
	use serde_json::json;

	fn db() -> (Db, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		(Db::open(&dir.path().join("r.db")).expect("open"), dir)
	}

	fn idea(db: &Db, id: &str, idea_type: IdeaType, parent: Option<&str>) {
		db.insert_idea(NewIdea {
			id,
			title: id,
			idea_type,
			result: "## A\na\n\n## B\nb",
			metadata: &json!({}),
			parent_idea_id: parent,
			..Default::default()
		})
		.expect("insert");
	}

	fn shared(section_index: i64, emoji: &str) -> SharedSectionReaction {
		SharedSectionReaction {
			section_index,
			emoji: emoji.into(),
		}
	}

	fn count(db: &Db, table: &str) -> i64 {
		db.lock()
			.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
			.unwrap()
	}

	#[test]
	fn migrates_v4_database_to_reaction_tables() {
		let dir = tempfile::tempdir().expect("tempdir");
		let path = dir.path().join("t.db");
		// a database exactly as schema version 4 left it
		{
			let conn = Connection::open(&path).unwrap();
			conn.execute_batch(BASELINE_SCHEMA).unwrap();
			conn.execute_batch("ALTER TABLE daily ADD COLUMN created_at TEXT NOT NULL DEFAULT '';")
				.unwrap();
			for table in ["ideas", "log_entries", "surveys"] {
				conn.execute_batch(&format!(
					"ALTER TABLE {table} ADD COLUMN local_date TEXT NOT NULL DEFAULT '';
					 CREATE INDEX idx_{table}_local_date ON {table}(local_date);"
				))
				.unwrap();
			}
			conn.execute_batch(
				"INSERT INTO ideas (id, created_at) VALUES ('old', '2026-01-01T00:00:00');",
			)
			.unwrap();
			conn.pragma_update(None, "user_version", 4).unwrap();
		}
		let db = Db::open(&path).expect("migrate v4 -> v5");
		{
			let conn = db.lock();
			let version: i64 = conn
				.query_row("PRAGMA user_version", [], |r| r.get(0))
				.unwrap();
			assert_eq!(version, 5);
			assert_eq!(version, SCHEMA_VERSION);
			for name in [
				"section_reactions",
				"comment_reactions",
				"idx_section_reactions_unique",
				"idx_section_reactions_source",
			] {
				let has: i64 = conn
					.query_row(
						"SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
						params![name],
						|r| r.get(0),
					)
					.unwrap();
				assert_eq!(has, 1, "{name} exists after the migration");
			}
		}
		// existing rows are untouched and the new tables are usable
		assert!(db.get_idea("old").unwrap().is_some());
		assert!(db.toggle_section_reaction("old", 1, "👍").unwrap());
		drop(db);
		// reopening a v5 file runs no step again
		let db = Db::open(&path).expect("reopen v5");
		assert_eq!(db.get_reactions("old").unwrap().sections.len(), 1);
	}

	#[test]
	fn section_reaction_toggles_on_and_off() {
		let (db, _dir) = db();
		idea(&db, "i", IdeaType::Original, None);
		assert!(db.toggle_section_reaction("i", 1, "👍").unwrap(), "now on");
		assert!(db.toggle_section_reaction("i", 1, "⚠️").unwrap());
		assert!(db.toggle_section_reaction("i", 2, "👍").unwrap());
		let mine = |section_index: i64, emoji: &str| SectionReaction {
			section_index,
			emoji: emoji.into(),
			mine: true,
			from: None,
		};
		assert_eq!(
			db.get_reactions("i").unwrap().sections,
			vec![mine(1, "👍"), mine(1, "⚠️"), mine(2, "👍")]
		);
		assert!(
			!db.toggle_section_reaction("i", 1, "👍").unwrap(),
			"now off"
		);
		assert_eq!(
			db.get_reactions("i").unwrap().sections,
			vec![mine(1, "⚠️"), mine(2, "👍")]
		);
	}

	#[test]
	fn section_reaction_writes_are_validated() {
		let (db, _dir) = db();
		idea(&db, "i", IdeaType::Original, None);
		let err = db.toggle_section_reaction("i", 0, "❤️").unwrap_err();
		assert!(err.contains("unsupported reaction"), "{err}");
		// the warning sign without its variation selector is not in the set
		assert!(db.toggle_section_reaction("i", 0, "\u{26A0}").is_err());
		let err = db.toggle_section_reaction("i", -1, "👍").unwrap_err();
		assert!(err.contains("invalid section index"), "{err}");
		let err = db.toggle_section_reaction("ghost", 0, "👍").unwrap_err();
		assert!(err.contains("not found"), "{err}");
		assert_eq!(count(&db, "section_reactions"), 0);
		// the schema backs the index check even for direct SQL
		let direct = db.lock().execute(
			"INSERT INTO section_reactions (idea_id, section_index, emoji, created_at) VALUES ('i', -1, '👍', 'x')",
			[],
		);
		assert!(direct.is_err(), "CHECK rejects a negative index");
	}

	#[test]
	fn comment_reactions_need_a_feedback_idea_and_show_on_its_parent() {
		let (db, _dir) = db();
		idea(&db, "i", IdeaType::Original, None);
		idea(&db, "f1", IdeaType::Feedback, Some("i"));
		idea(&db, "f2", IdeaType::Feedback, Some("i"));
		idea(&db, "other", IdeaType::Original, None);
		idea(&db, "f3", IdeaType::Feedback, Some("other"));

		let err = db.toggle_comment_reaction("i", 0, "👍").unwrap_err();
		assert!(err.contains("not feedback"), "{err}");
		let err = db.toggle_comment_reaction("ghost", 0, "👍").unwrap_err();
		assert!(err.contains("not found"), "{err}");
		let err = db.toggle_comment_reaction("f1", -1, "👍").unwrap_err();
		assert!(err.contains("invalid comment index"), "{err}");
		assert!(db.toggle_comment_reaction("f1", 0, "nope").is_err());
		assert_eq!(count(&db, "comment_reactions"), 0);

		assert!(db.toggle_comment_reaction("f1", 0, "👍").unwrap());
		assert!(db.toggle_comment_reaction("f1", 3, "🚀").unwrap());
		assert!(db.toggle_comment_reaction("f2", 0, "❓").unwrap());
		assert!(db.toggle_comment_reaction("f3", 0, "📚").unwrap());
		assert!(!db.toggle_comment_reaction("f1", 3, "🚀").unwrap(), "off");

		let comment = |feedback_idea_id: &str, item_index: i64, emoji: &str| CommentReaction {
			feedback_idea_id: feedback_idea_id.into(),
			item_index,
			emoji: emoji.into(),
		};
		let reactions = db.get_reactions("i").unwrap();
		assert!(reactions.sections.is_empty());
		assert_eq!(
			reactions.comments,
			vec![comment("f1", 0, "👍"), comment("f2", 0, "❓")],
			"only this idea's feedback children"
		);
		assert_eq!(
			db.get_reactions("other").unwrap().comments,
			vec![comment("f3", 0, "📚")]
		);
		assert_eq!(db.get_reactions("ghost").unwrap(), IdeaReactions::default());
	}

	#[test]
	fn imported_reactions_are_attributed_to_their_feedback() {
		let (db, _dir) = db();
		idea(&db, "i", IdeaType::Original, None);
		assert!(db.toggle_section_reaction("i", 1, "👍").unwrap());
		let carried = [shared(1, "👍"), shared(2, "💡"), shared(2, "💡")];
		db.insert_idea(NewIdea::imported(NewIdea {
			id: "fb",
			title: "Feedback",
			idea_type: IdeaType::Feedback,
			result: "r",
			metadata: &json!({ "imported": true }),
			parent_idea_id: Some("i"),
			creator_name: Some("Grace"),
			parent_section_reactions: &carried,
			..Default::default()
		}))
		.expect("insert imported feedback");
		let grace = |section_index: i64, emoji: &str| SectionReaction {
			section_index,
			emoji: emoji.into(),
			mine: false,
			from: Some("Grace".into()),
		};
		// the same emoji from me and from Grace are separate reactions;
		// the repeated entry was stored once
		assert_eq!(
			db.get_reactions("i").unwrap().sections,
			vec![
				SectionReaction {
					section_index: 1,
					emoji: "👍".into(),
					mine: true,
					from: None,
				},
				grace(1, "👍"),
				grace(2, "💡"),
			]
		);
		// toggling my own reaction never touches the imported one
		assert!(!db.toggle_section_reaction("i", 1, "👍").unwrap());
		assert_eq!(
			db.get_reactions("i").unwrap().sections,
			vec![grace(1, "👍"), grace(2, "💡")]
		);
		// only my own reactions are what an export carries
		assert!(db.my_section_reactions("i").unwrap().is_empty());
		assert!(db.toggle_section_reaction("i", 3, "🚀").unwrap());
		assert_eq!(db.my_section_reactions("i").unwrap(), vec![shared(3, "🚀")]);
	}

	#[test]
	fn invalid_carried_reactions_reject_the_whole_insert() {
		let (db, _dir) = db();
		idea(&db, "i", IdeaType::Original, None);
		for bad in [shared(0, "❤️"), shared(-1, "👍")] {
			let carried = [shared(0, "👍"), bad];
			db.insert_idea(NewIdea {
				id: "fb",
				title: "F",
				idea_type: IdeaType::Feedback,
				result: "r",
				metadata: &json!({}),
				parent_idea_id: Some("i"),
				parent_section_reactions: &carried,
				..Default::default()
			})
			.expect_err("invalid reaction must fail the insert");
			assert!(db.get_idea("fb").unwrap().is_none(), "rolled back");
			assert_eq!(count(&db, "section_reactions"), 0, "rolled back");
		}
		// reactions without a parent to attach to are refused
		let carried = [shared(0, "👍")];
		db.insert_idea(NewIdea {
			id: "lone",
			title: "L",
			idea_type: IdeaType::Original,
			result: "r",
			metadata: &json!({}),
			parent_section_reactions: &carried,
			..Default::default()
		})
		.expect_err("no parent to attach to");
		assert!(db.get_idea("lone").unwrap().is_none());
	}

	#[test]
	fn deleting_ideas_cascades_to_their_reactions() {
		let (db, _dir) = db();
		idea(&db, "i", IdeaType::Original, None);
		idea(&db, "mine-fb", IdeaType::Feedback, Some("i"));
		let carried = [shared(1, "👍")];
		db.insert_idea(NewIdea {
			id: "their-fb",
			title: "F",
			idea_type: IdeaType::Feedback,
			result: "r",
			metadata: &json!({}),
			parent_idea_id: Some("i"),
			creator_name: Some("Grace"),
			parent_section_reactions: &carried,
			..Default::default()
		})
		.unwrap();
		db.toggle_section_reaction("i", 1, "👍").unwrap();
		db.toggle_comment_reaction("their-fb", 0, "👍").unwrap();
		db.toggle_comment_reaction("mine-fb", 0, "❓").unwrap();
		assert_eq!(count(&db, "section_reactions"), 2);
		assert_eq!(count(&db, "comment_reactions"), 2);

		// deleting one feedback removes the section reactions it carried
		// and the comment reactions on it, nothing else
		assert!(db.delete_idea("their-fb").unwrap());
		let reactions = db.get_reactions("i").unwrap();
		assert_eq!(reactions.sections.len(), 1);
		assert!(reactions.sections[0].mine);
		assert_eq!(reactions.comments.len(), 1);
		assert_eq!(reactions.comments[0].feedback_idea_id, "mine-fb");

		// deleting the idea removes everything left
		assert!(db.delete_idea("i").unwrap());
		assert_eq!(count(&db, "section_reactions"), 0);
		assert_eq!(count(&db, "comment_reactions"), 0);
	}
}
