use std::path::Path;
use std::sync::Mutex;

use chrono::{NaiveDate, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension};

use crate::keys::setting::USER_TIMEZONE;
use crate::reactions::SharedSectionReaction;
use crate::types::{ChatMessage, IdeaType};

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

mod daily;
mod ideas;
mod migrations;
mod reactions;

use migrations::SCHEMA_VERSION;

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
	/// written as `is_unread` in the same insert (imported feedback
	/// arrives unread; no second statement can fail after the row exists)
	pub unread: bool,
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
		// The settings table can hold plaintext secrets when no keychain
		// exists: keep the database (and its journal/WAL sidecars, which
		// inherit the file's mode) owner-only. Best effort on exotic
		// filesystems.
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
		}
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
		// Schema steps and the every-open self-healing pass
		// live in the migrations module.
		migrations::migrate(&mut conn, version)?;
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

	pub(super) fn table_has_column(
		conn: &Connection,
		table: &str,
		column: &str,
	) -> rusqlite::Result<bool> {
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

	/// Atomically flip a boolean setting (stored as "true"/"false") and
	/// return the new value: the tray reminder toggle's read-modify-write
	/// cannot be split by, or race, a concurrent settings save.
	pub fn toggle_setting_bool(&self, key: &str, default: bool) -> Result<bool, String> {
		let mut conn = self.lock();
		let tx = conn.transaction().map_err(|e| e.to_string())?;
		let current: Option<String> = tx
			.query_row(
				"SELECT value FROM settings WHERE key = ?1",
				params![key],
				|row| row.get(0),
			)
			.optional()
			.map_err(|e| format!("failed to read setting '{key}': {e}"))?;
		let next = !current.as_deref().map(|v| v == "true").unwrap_or(default);
		tx.execute(
			"INSERT INTO settings (key, value) VALUES (?1, ?2)
			 ON CONFLICT(key) DO UPDATE SET value = ?2",
			params![key, if next { "true" } else { "false" }],
		)
		.map_err(|e| format!("failed to save setting '{key}': {e}"))?;
		tx.commit().map_err(|e| e.to_string())?;
		Ok(next)
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
}

#[cfg(test)]
mod coverage_tests;
#[cfg(test)]
mod reaction_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod timezone_tests;
#[cfg(test)]
mod v4_tests;
