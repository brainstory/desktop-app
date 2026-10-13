//! The schema steps (each one transaction, committed with its
//! `user_version` bump) and the every-open maintenance pass.

use rusqlite::Connection;

use super::{own_activity_filter, Db, OpenError};

pub(super) const BASELINE_SCHEMA: &str = "
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
pub(super) const REACTIONS_SCHEMA: &str = "
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
///   6: lookup indexes the first five missed: `ideas(created_at)` (the
///      library grid's ORDER BY on every dashboard load) and
///      `surveys(idea_id)` (cleared per idea on a delete)
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
pub(super) const SCHEMA_VERSION: i64 = 6;

/// Bring an open connection from `version` to [`SCHEMA_VERSION`] and
/// run the self-healing maintenance (local_date backfill, dangling
/// soft references, JSON validity reporting). The newer-schema refusal
/// stays in `Db::open`, before this runs.
pub(super) fn migrate(conn: &mut Connection, version: i64) -> Result<(), OpenError> {
	let _ = version;
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
		if !Db::table_has_column(&tx, "daily", "created_at")? {
			tx.execute_batch("ALTER TABLE daily ADD COLUMN created_at TEXT NOT NULL DEFAULT '';")?;
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
			if !Db::table_has_column(&tx, table, "local_date")? {
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
	if version < 6 {
		// The library grid orders by created_at on every dashboard
		// load, and a delete clears surveys by idea_id; both had no
		// index to stand on.
		let tx = conn.transaction()?;
		tx.execute_batch(
			"CREATE INDEX IF NOT EXISTS idx_ideas_created_at ON ideas(created_at);
				 CREATE INDEX IF NOT EXISTS idx_surveys_idea_id ON surveys(idea_id);",
		)?;
		tx.pragma_update(None, "user_version", 6)?;
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
	// Self-healing soft references: daily.log_id, daily.intent_idea_id,
	// daily.survey_id and surveys.idea_id have no foreign key (the
	// delete paths clear them transactionally), so a crash mid-delete
	// or a hand edit can leave them dangling. Clearing them at open
	// keeps the dashboard from linking to a ghost.
	conn.execute_batch(
		"UPDATE daily SET intent_idea_id = NULL
			   WHERE intent_idea_id IS NOT NULL AND intent_idea_id NOT IN (SELECT id FROM ideas);
			 UPDATE daily SET log_id = NULL
			   WHERE log_id IS NOT NULL AND log_id NOT IN (SELECT id FROM log_entries);
			 UPDATE daily SET survey_id = NULL
			   WHERE survey_id IS NOT NULL AND survey_id NOT IN (SELECT id FROM surveys);
			 UPDATE surveys SET idea_id = NULL
			   WHERE idea_id IS NOT NULL AND idea_id NOT IN (SELECT id FROM ideas);",
	)?;
	// The JSON columns are written from already-parsed values, so a
	// row failing json_valid can only come from a hand edit or a
	// foreign tool. The read paths guard with json_valid; report the
	// count here so a broken store leaves a trace instead of silently
	// reading as empty.
	for (table, column, nullable) in [
		("ideas", "idea_metadata", false),
		("ideas", "structured_result", true),
		("ideas", "transcript", false),
		("log_entries", "answers", false),
		("surveys", "answers", false),
	] {
		let guard = if nullable {
			format!("{column} IS NOT NULL AND ")
		} else {
			String::new()
		};
		let count: i64 = conn
			.query_row(
				&format!("SELECT COUNT(*) FROM {table} WHERE {guard}NOT json_valid({column})"),
				[],
				|row| row.get(0),
			)
			.unwrap_or(0);
		if count > 0 {
			log::warn!(
					"{count} {table}.{column} row(s) are not valid JSON; reads degrade to empty for them"
				);
		}
	}
	Ok(())
}
