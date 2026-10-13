use super::migrations::BASELINE_SCHEMA;
use super::*;
use crate::reactions::{CommentReaction, IdeaReactions, SectionReaction, SharedSectionReaction};
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
