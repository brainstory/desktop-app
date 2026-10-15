use super::migrations::{BASELINE_SCHEMA, REACTIONS_SCHEMA};
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
			conn.execute_batch("ALTER TABLE daily ADD COLUMN created_at TEXT NOT NULL DEFAULT '';")
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
		assert!(
			db.has_activity_today().expect("activity check"),
			"v{version}: today's log counts"
		);
		assert_eq!(
			db.get_daily_status().expect("daily status").streak,
			1,
			"v{version}: the backfilled log feeds the daily intent streak"
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

#[test]
fn migration_v6_adds_the_created_at_and_survey_indexes() {
	let dir = tempfile::tempdir().expect("tempdir");
	let path = dir.path().join("t.db");
	// a database exactly as schema version 5 left it
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
		conn.execute_batch(REACTIONS_SCHEMA).unwrap();
		conn.pragma_update(None, "user_version", 5).unwrap();
	}
	let db = Db::open(&path).expect("migrate v5 -> v6");
	{
		let conn = db.lock();
		let version: i64 = conn
			.query_row("PRAGMA user_version", [], |r| r.get(0))
			.unwrap();
		assert_eq!(version, SCHEMA_VERSION);
		for index in ["idx_ideas_created_at", "idx_surveys_idea_id"] {
			let has: i64 = conn
				.query_row(
					"SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
					params![index],
					|r| r.get(0),
				)
				.unwrap();
			assert_eq!(has, 1, "{index} exists");
		}
	}
}
