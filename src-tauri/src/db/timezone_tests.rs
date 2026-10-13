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
