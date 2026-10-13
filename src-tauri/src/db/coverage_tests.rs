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
	// the streak counts daily intent activity: submit a daily log on
	// the given day offset
	let log_on = |offset: i64| {
		let date = (db.today_local() - chrono::Duration::days(offset))
			.format("%Y-%m-%d")
			.to_string();
		db.lock()
				.execute(
					"INSERT INTO log_entries (id, answers, created_at, local_date) VALUES (?1, '[]', ?2, ?3)",
					rusqlite::params![format!("l{offset}"), ts(offset), date],
				)
				.unwrap();
	};
	// today, yesterday, 2 days ago; a hole on day 3; activity on day 4
	log_on(0);
	log_on(1);
	log_on(2);
	log_on(4);
	assert_eq!(db.get_daily_status().streak, 3, "stops at the gap");

	// an ordinary idea fills no gap: it is not daily intent activity
	idea(&db, "bridge", &ts(3));
	assert_eq!(
		db.get_daily_status().streak,
		3,
		"an ordinary idea must not bridge the gap"
	);
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
