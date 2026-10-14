use super::migrations::BASELINE_SCHEMA;
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
	assert_eq!(
		status.streak, 0,
		"the retired survey flow does not feed the daily intent streak"
	);
}

#[test]
fn toggle_setting_bool_flips_from_stored_or_default() {
	let (path, _dir) = temp_db_path();
	let db = Db::open(&path).expect("open");
	// absent: starts from the default (false) -> true
	assert!(db.toggle_setting_bool("k1", false).unwrap());
	assert_eq!(db.get_setting("k1").as_deref(), Some("true"));
	// stored: flips the stored value
	assert!(!db.toggle_setting_bool("k1", false).unwrap());
	assert_eq!(db.get_setting("k1").as_deref(), Some("false"));
	// an unrelated stored string is not "true"
	db.set_setting("k2", "junk").unwrap();
	assert!(db.toggle_setting_bool("k2", false).unwrap());
	assert_eq!(db.get_setting("k2").as_deref(), Some("true"));
}

#[test]
fn link_idea_log_attaches_and_relinks_a_daily_intent() {
	let (path, _dir) = temp_db_path();
	let db = Db::open(&path).expect("open");
	let draft = vec![ChatMessage {
		role: "user".into(),
		content: "talked through the day".into(),
	}];
	// the session existed before today's log did: no log link
	db.create_daily_intent_idea("i1", "", "", &draft, &serde_json::json!({}))
		.unwrap();
	let linked_log = || {
		db.lock()
			.query_row("SELECT log_id FROM ideas WHERE id = 'i1'", [], |row| {
				row.get::<_, Option<String>>(0)
			})
			.unwrap()
	};
	assert_eq!(linked_log(), None);

	// the log submitted afterwards links the existing idea
	db.link_idea_log("i1", "log-1").unwrap();
	assert_eq!(linked_log().as_deref(), Some("log-1"));

	// a later log (a re-submitted same-day log) relinks
	db.link_idea_log("i1", "log-2").unwrap();
	assert_eq!(linked_log().as_deref(), Some("log-2"));

	// unknown ideas report instead of silently no-op'ing
	let err = db
		.link_idea_log("ghost", "log-1")
		.expect_err("missing idea");
	assert!(err.contains("not found"), "unexpected error: {err}");
}

#[test]
fn quiesce_waits_for_a_held_connection_and_returns_after_release() {
	let (path, _dir) = temp_db_path();
	let db = Db::open(&path).expect("open");
	// one write so the WAL has something to checkpoint
	db.set_setting("k", "v").unwrap();

	// Hold the connection like an in-flight write would (a MutexGuard
	// is not Send, so the holder stays on this thread); quiesce runs on
	// a scoped thread and must return only once the hold ends - not
	// via its own timeout (which is 2s, far past the 250ms hold).
	let started = std::time::Instant::now();
	std::thread::scope(|scope| {
		let _guard = db.conn.try_lock().expect("uncontended");
		let waiter = scope.spawn(|| db.quiesce(std::time::Duration::from_secs(2)));
		std::thread::sleep(std::time::Duration::from_millis(250));
		drop(_guard);
		waiter
			.join()
			.expect("quiesce returns once the connection is released");
	});
	let elapsed = started.elapsed();
	assert!(
		elapsed >= std::time::Duration::from_millis(200),
		"quiesce must wait out the held connection, returned after {elapsed:?}"
	);
	assert!(
		elapsed < std::time::Duration::from_secs(2),
		"quiesce returned via its timeout instead of the release ({elapsed:?})"
	);

	// uncontended: returns promptly (and checkpoints) instead of
	// sleeping the timeout away
	let again = std::time::Instant::now();
	db.quiesce(std::time::Duration::from_secs(2));
	assert!(again.elapsed() < std::time::Duration::from_secs(1));
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
fn dangling_soft_references_are_cleared_on_open() {
	let (path, _dir) = temp_db_path();
	{
		let db = Db::open(&path).expect("open");
		// rows pointing at ids that do not exist (a crash mid-delete
		// or a hand edit)
		db.lock()
			.execute_batch(
				"INSERT INTO daily (date, log_id, intent_idea_id, survey_id, is_completed)
					   VALUES ('2026-01-02', 'ghost-log', 'ghost-idea', 'ghost-survey', 0);
					 INSERT INTO surveys (id, idea_id, answers, created_at)
					   VALUES ('s1', 'ghost-idea', '{}', '2026-01-02T00:00:00');",
			)
			.unwrap();
	}
	let db = Db::open(&path).expect("reopen");
	let conn = db.lock();
	let (log_id, intent, survey): (Option<String>, Option<String>, Option<String>) = conn
		.query_row(
			"SELECT log_id, intent_idea_id, survey_id FROM daily WHERE date = '2026-01-02'",
			[],
			|r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
		)
		.unwrap();
	assert_eq!(log_id, None, "dangling log reference cleared");
	assert_eq!(intent, None, "dangling intent reference cleared");
	assert_eq!(survey, None, "dangling survey reference cleared");
	let survey_idea: Option<String> = conn
		.query_row("SELECT idea_id FROM surveys WHERE id = 's1'", [], |r| {
			r.get(0)
		})
		.unwrap();
	assert_eq!(survey_idea, None, "dangling survey->idea reference cleared");
}

#[test]
fn invalid_json_rows_are_reported_not_fatal() {
	let (path, _dir) = temp_db_path();
	{
		let db = Db::open(&path).expect("open");
		db.lock()
			.execute_batch(
				"INSERT INTO ideas (id, title, result, idea_metadata, transcript, created_at)
					   VALUES ('broken', 'Broken', '', 'not-json', 'also not json', '2026-01-02T00:00:00');",
			)
			.unwrap();
	}
	// the open reports the row (logged) instead of failing, and every
	// guarded query still reads past it
	let db = Db::open(&path).expect("reopen");
	let ideas = db.list_ideas().expect("list");
	assert_eq!(ideas.len(), 1);
	assert_eq!(ideas[0].id, "broken");
	// the activity filter's json_valid CASE returns false, not an error
	let _ = db.has_activity_today();
	let _ = db.get_daily_status();
}

#[test]
fn notes_are_structured_and_rebuild_one_trailing_section() {
	let (path, _dir) = temp_db_path();
	let db = Db::open(&path).expect("open");
	db.insert_idea(NewIdea {
		id: "n1",
		title: "T",
		idea_type: IdeaType::Original,
		result: "# Title\n\nbody",
		metadata: &serde_json::json!({}),
		..Default::default()
	})
	.unwrap();

	db.add_idea_note("n1", "first").unwrap();
	let idea = db.get_idea("n1").unwrap().unwrap();
	let result = idea.result.clone().unwrap();
	assert!(result.starts_with("# Title\n\nbody"), "writeup untouched");
	assert!(
		result.ends_with("## Notes\n\nfirst"),
		"note under a heading"
	);
	let notes = idea.notes.expect("structured notes");
	assert_eq!(notes.len(), 1);
	assert_eq!(notes[0].text, "first");
	let first_id = notes[0].id.clone();

	db.add_idea_note("n1", "second").unwrap();
	let result = db.get_idea("n1").unwrap().unwrap().result.unwrap();
	assert!(result.ends_with("## Notes\n\nfirst\n\nsecond"));
	assert_eq!(
		result.matches("## Notes").count(),
		1,
		"one notes section, never one per note"
	);

	// edit the first note: the rendered section follows the structured
	// list, and the other note is untouched
	db.update_idea_note("n1", &first_id, "first fixed").unwrap();
	let idea = db.get_idea("n1").unwrap().unwrap();
	let result = idea.result.unwrap();
	assert!(result.ends_with("## Notes\n\nfirst fixed\n\nsecond"));
	let second_id = idea.notes.as_ref().unwrap()[1].id.clone();
	assert_eq!(idea.notes.unwrap()[0].text, "first fixed");

	// deleting the last note removes the section entirely
	db.delete_idea_note("n1", &first_id).unwrap();
	db.delete_idea_note("n1", &second_id).unwrap();
	let idea = db.get_idea("n1").unwrap().unwrap();
	assert_eq!(idea.result.as_deref(), Some("# Title\n\nbody"));
	assert!(idea.notes.is_none(), "no notes left");

	// unknown note ids are an error, not a silent no-op
	let err = db
		.update_idea_note("n1", "ghost", "x")
		.expect_err("updating a missing note must fail");
	assert!(err.contains("note not found"), "unexpected error: {err}");
	let err = db
		.delete_idea_note("n1", "ghost")
		.expect_err("deleting a missing note must fail");
	assert!(err.contains("note not found"), "unexpected error: {err}");

	// an empty result still gets a well-formed notes section
	db.insert_idea(NewIdea {
		id: "n2",
		title: "T",
		idea_type: IdeaType::Original,
		result: "",
		metadata: &serde_json::json!({}),
		..Default::default()
	})
	.unwrap();
	db.add_idea_note("n2", "only a note").unwrap();
	assert_eq!(
		db.get_idea("n2").unwrap().unwrap().result.as_deref(),
		Some("## Notes\n\nonly a note")
	);

	// notes are not the finish gesture: a note on a daily intent does
	// not complete the day
	db.create_daily_intent_idea("i1", "", "", &[], &serde_json::json!({}))
		.unwrap();
	db.add_idea_note("i1", "mid-day thought").unwrap();
	assert!(!db.get_daily_status().is_completed);

	let err = db
		.add_idea_note("ghost", "x")
		.expect_err("noting a missing idea must fail");
	assert!(err.contains("not found"), "unexpected error: {err}");
}

#[test]
fn the_first_note_adopts_the_append_only_notes_section() {
	// an idea written by the old append-only writer: notes in markdown,
	// no structured metadata yet
	let (path, _dir) = temp_db_path();
	let db = Db::open(&path).expect("open");
	db.insert_idea(NewIdea {
		id: "legacy",
		title: "T",
		idea_type: IdeaType::Original,
		result: "# Title\n\nbody\n\n## Notes\n\nan old note\n\nanother old note",
		metadata: &serde_json::json!({}),
		..Default::default()
	})
	.unwrap();

	// reading keeps them in the writeup: nothing is rewritten until a
	// mutation happens
	let idea = db.get_idea("legacy").unwrap().unwrap();
	assert!(idea.notes.is_none(), "nothing structured yet");
	assert!(idea.result.as_deref().unwrap().contains("an old note"));

	// the first mutation adopts them (and then adds the new note)
	db.add_idea_note("legacy", "new note").unwrap();
	let idea = db.get_idea("legacy").unwrap().unwrap();
	let notes = idea.notes.expect("adopted notes");
	assert_eq!(notes.len(), 3);
	assert_eq!(notes[0].text, "an old note");
	assert_eq!(notes[1].text, "another old note");
	assert_eq!(notes[2].text, "new note");
	let result = idea.result.unwrap();
	assert_eq!(
		result.matches("## Notes").count(),
		1,
		"adoption rebuilds the one section"
	);
	assert!(result.ends_with("## Notes\n\nan old note\n\nanother old note\n\nnew note"));
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
				"INSERT INTO ideas (id, created_at) VALUES ('old', strftime('%Y-%m-%dT%H:%M:%S', 'now', '-1 day'));
				 INSERT INTO log_entries (id, answers, created_at) VALUES ('old-log', '[]', strftime('%Y-%m-%dT%H:%M:%S', 'now', '-1 day'));",
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
	// the backfilled log row counts toward the daily intent streak
	// again, not just after the healing open but on every later open too
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
				"INSERT INTO ideas (id, created_at) VALUES ('old', strftime('%Y-%m-%dT%H:%M:%S', 'now', '-1 day'));
				 INSERT INTO log_entries (id, answers, created_at) VALUES ('old-log', '[]', strftime('%Y-%m-%dT%H:%M:%S', 'now', '-1 day'));",
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
	// yesterday's log counts via its frozen local date; today without
	// daily intent activity doesn't break the streak
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
