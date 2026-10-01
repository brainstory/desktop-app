use serde_json::json;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::types::IdeaType;
use crate::AppState;

// Trust model: share files are plain JSON the user received however they
// liked (email, AirDrop, ...). Nothing cryptographically ties a file to an
// author - a crafted file can claim any author name or share id (which can
// suppress a later import of the real file as a "duplicate"). That is an
// accepted property of this peer-to-peer, trust-based flow; the surfaces
// below still bound file/content sizes so a hostile file can only confuse,
// not exhaust, the app.

/// Largest share file we will read into memory.
const MAX_SHARE_FILE_BYTES: u64 = 10 * 1024 * 1024;
/// Largest structured feedback document we will store.
const MAX_STRUCTURED_BYTES: usize = 1024 * 1024;
/// Longest title accepted from a share file (longer means the file is
/// hostile, not chatty).
const MAX_TITLE_CHARS: usize = 200;
/// Longest result document accepted from a share file.
const MAX_RESULT_BYTES: usize = 1024 * 1024;

pub const SHARE_FORMAT: &str = "brainstory-share";
pub const SHARE_VERSION: i64 = 1;

/// The versioned content of a share file, independent of how it travels.
/// `build_export_payload` turns this into JSON; `parse_share_payload`
/// validates untrusted bytes back into it. Both are pure so the
/// export -> import contract can be unit-tested end to end.
#[derive(Debug, Clone, PartialEq)]
pub enum SharePayload {
	Idea {
		share_id: String,
		title: String,
		result: String,
		idea_type: IdeaType,
		created_at: Option<String>,
	},
	Feedback {
		target_share_id: String,
		target_title: String,
		title: String,
		result: String,
		structured_result: Option<serde_json::Value>,
		created_at: Option<String>,
	},
}

impl SharePayload {
	pub fn kind_label(&self) -> &'static str {
		match self {
			SharePayload::Idea { .. } => "idea",
			SharePayload::Feedback { .. } => "feedback",
		}
	}
}

/// The share's author plus its payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedShare {
	pub author: String,
	pub payload: SharePayload,
}

/// Serialize a share payload into the portable JSON envelope.
pub fn build_export_payload(author: &str, payload: &SharePayload) -> serde_json::Value {
	let mut root = json!({
		"format": SHARE_FORMAT,
		"version": SHARE_VERSION,
		"exported_at": chrono::Utc::now().to_rfc3339(),
		"author": author,
	});
	match payload {
		SharePayload::Idea {
			share_id,
			title,
			result,
			idea_type,
			created_at,
		} => {
			root["kind"] = json!("idea");
			root["idea"] = json!({
				"share_id": share_id,
				"title": title,
				"result": result,
				"type": idea_type,
				"created_at": created_at,
			});
		}
		SharePayload::Feedback {
			target_share_id,
			target_title,
			title,
			result,
			structured_result,
			created_at,
		} => {
			root["kind"] = json!("feedback");
			root["feedback"] = json!({
				"target_share_id": target_share_id,
				"target_title": target_title,
				"title": title,
				"result": result,
				"structured_result": structured_result,
				"created_at": created_at,
			});
		}
	}
	root
}

/// Prefer the original creation date; malformed strings fall back to now.
fn parse_created_at(value: Option<&str>) -> Option<String> {
	let value = value?;
	chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
		.ok()
		.map(|_| value.to_string())
}

/// Validate untrusted share-file bytes into a typed payload. Missing or
/// wrongly-typed optional fields fall back to defaults; structural
/// problems (bad JSON, wrong format, unknown version/kind) are errors.
pub fn parse_share_payload(raw: &str) -> Result<ParsedShare, String> {
	let root: serde_json::Value =
		serde_json::from_str(raw).map_err(|e| format!("not a valid share file: {e}"))?;
	if root["format"].as_str() != Some(SHARE_FORMAT) {
		return Err("not a brainstory share file".into());
	}
	match root["version"].as_i64() {
		Some(v) if v == SHARE_VERSION => {}
		Some(v) if v > SHARE_VERSION => {
			return Err(format!(
				"share file version {v} is newer than this app supports - update Brainstory and try again"
			))
		}
		Some(v) => {
			return Err(format!(
				"share file version {v} is older than this app supports - please re-export"
			))
		}
		None => return Err("share file is missing its version".into()),
	}
	let author = root["author"]
		.as_str()
		.filter(|s| !s.is_empty())
		.unwrap_or("Anonymous")
		.to_string();
	let kind = root["kind"]
		.as_str()
		.ok_or_else(|| "share file is missing its kind".to_string())?;

	let payload = match kind {
		"idea" => {
			let idea = &root["idea"];
			let title = idea["title"].as_str().unwrap_or("Imported idea");
			let result = idea["result"].as_str().unwrap_or("");
			let idea_type = idea["type"].as_str().unwrap_or(IdeaType::Original.as_str());
			validate_imported_fields(title, result)?;
			if idea_type != IdeaType::Original.as_str() {
				// A share file can claim any type string; only original
				// ideas exist in exports, so anything else is a crafted
				// file trying to smuggle e.g. "daily_intent" into the
				// library.
				return Err(format!(
					"share files can only contain original ideas, not '{idea_type}'"
				));
			}
			SharePayload::Idea {
				share_id: idea["share_id"].as_str().unwrap_or("").to_string(),
				title: title.to_string(),
				result: result.to_string(),
				idea_type: IdeaType::Original,
				created_at: parse_created_at(idea["created_at"].as_str()),
			}
		}
		"feedback" => {
			let feedback = &root["feedback"];
			let title = feedback["title"].as_str().unwrap_or("");
			let result = feedback["result"].as_str().unwrap_or("");
			validate_imported_fields(title, result)?;
			SharePayload::Feedback {
				target_share_id: feedback["target_share_id"]
					.as_str()
					.unwrap_or("")
					.to_string(),
				target_title: feedback["target_title"].as_str().unwrap_or("").to_string(),
				title: title.to_string(),
				result: result.to_string(),
				// only objects are accepted as structured documents
				structured_result: feedback["structured_result"]
					.as_object()
					.map(|_| feedback["structured_result"].clone()),
				created_at: parse_created_at(feedback["created_at"].as_str()),
			}
		}
		other => return Err(format!("unknown share kind: {other}")),
	};
	Ok(ParsedShare { author, payload })
}

/// Titles and results from untrusted files are bounded independently of
/// the file-size limit so they cannot flood the library UI.
fn validate_imported_fields(title: &str, result: &str) -> Result<(), String> {
	if title.chars().count() > MAX_TITLE_CHARS {
		return Err(format!(
			"share file title is too long (limit {MAX_TITLE_CHARS} characters)"
		));
	}
	if result.len() > MAX_RESULT_BYTES {
		return Err(format!(
			"share file result is too large (limit {} bytes)",
			MAX_RESULT_BYTES
		));
	}
	Ok(())
}

/// The author and payload an export of `idea_id` carries. Split out of
/// the command so the export -> import round trip can be tested without
/// a native save dialog.
fn export_share(db: &crate::db::Db, idea_id: &str) -> Result<(String, SharePayload), String> {
	let idea = db
		.get_idea(idea_id)?
		.ok_or_else(|| format!("idea {idea_id} not found"))?;

	let is_feedback = idea.r#type.as_deref() == Some(IdeaType::Feedback.as_str());
	let own_name = db
		.get_setting(crate::keys::setting::USER_NAME)
		.filter(|s| !s.is_empty());
	let author = idea
		.creator_name
		.clone()
		.or(own_name)
		.unwrap_or_else(|| "Anonymous".into());

	let payload = if is_feedback {
		let (target_share_id, target_title) = match &idea.parent_idea {
			Some(parent) => (
				db.get_share_id(&parent.id)
					.unwrap_or_else(|| parent.id.clone()),
				parent.title.clone(),
			),
			None => (String::new(), String::new()),
		};
		SharePayload::Feedback {
			target_share_id,
			target_title,
			title: idea.title.clone(),
			result: idea.result.clone().unwrap_or_default(),
			structured_result: idea.structured_result.clone(),
			created_at: Some(idea.created_at.clone()),
		}
	} else {
		let share_id = db.get_share_id(&idea.id).unwrap_or_else(|| idea.id.clone());
		SharePayload::Idea {
			share_id,
			title: idea.title.clone(),
			result: idea.result.clone().unwrap_or_default(),
			// Every non-feedback idea travels as "original": a daily
			// intent is the author's own day plan, meaningless as the
			// reader's, and the import whitelist only accepts originals.
			idea_type: IdeaType::Original,
			created_at: Some(idea.created_at.clone()),
		}
	};
	Ok((author, payload))
}

/// Export an idea (or a feedback document) as a portable JSON file. The user
/// sends the file to the other person however they like; importing it on
/// another machine attributes the content to the author stored in the file.
/// The chat transcript is deliberately NOT included — only the distilled
/// result is shared, so the raw conversation stays on the author's machine.
#[tauri::command]
pub async fn export_idea(
	app: tauri::AppHandle,
	state: State<'_, AppState>,
	idea_id: String,
) -> Result<serde_json::Value, String> {
	let (author, payload) = export_share(&state.db, &idea_id)?;
	let envelope = build_export_payload(&author, &payload);
	let file_tag = match &payload {
		SharePayload::Idea { idea_type, .. } => idea_type.as_str(),
		SharePayload::Feedback { .. } => "feedback",
	};
	let default_name = format!(
		"brainstory-{}-{}.json",
		file_tag,
		chrono::Local::now().format("%Y%m%d-%H%M")
	);

	let (sender, receiver) = tokio::sync::oneshot::channel();
	let dialog = app
		.dialog()
		.file()
		.set_file_name(&default_name)
		.add_filter("Brainstory share", &["json"]);
	dialog.save_file(move |path| {
		let _ = sender.send(path);
	});
	let path = receiver.await.map_err(|e| e.to_string())?;
	let Some(path) = path else {
		// user cancelled the dialog
		return Ok(json!({ "cancelled": true }));
	};
	let path = path.into_path().map_err(|e| e.to_string())?;

	let written = serde_json::to_string_pretty(&envelope).unwrap_or_default();
	std::fs::write(&path, written).map_err(|e| e.to_string())?;

	Ok(json!({
		"cancelled": false,
		"path": path.to_string_lossy(),
		"kind": payload.kind_label(),
	}))
}

/// Import a previously exported brainstory share file. Importing an idea adds
/// it to the library attributed to its author; importing feedback attaches it
/// to the matching local idea, attributed to the feedback author and marked
/// unread.
#[tauri::command]
pub async fn import_share(
	app: tauri::AppHandle,
	state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
	let (sender, receiver) = tokio::sync::oneshot::channel();
	app.dialog()
		.file()
		.add_filter("Brainstory share", &["json"])
		.pick_file(move |path| {
			let _ = sender.send(path);
		});
	let path = receiver.await.map_err(|e| e.to_string())?;
	let Some(path) = path else {
		return Ok(json!({ "cancelled": true }));
	};
	let path = path.into_path().map_err(|e| e.to_string())?;

	// Bound the read: a corrupt or hostile multi-GB "share file" must not
	// spike memory (the dialog filter is only a hint; nothing enforces it).
	let file_len = std::fs::metadata(&path)
		.map_err(|e| format!("could not read file: {e}"))?
		.len();
	if file_len > MAX_SHARE_FILE_BYTES {
		return Err(format!(
			"share file is too large ({} MB, limit {} MB)",
			file_len / (1024 * 1024),
			MAX_SHARE_FILE_BYTES / (1024 * 1024)
		));
	}

	let raw = std::fs::read_to_string(&path).map_err(|e| format!("could not read file: {e}"))?;
	let parsed = parse_share_payload(&raw)?;
	import_parsed(&state.db, parsed)
}

/// Everything after the file dialog: dedupe, attach and store. Split out
/// of the command so the untrusted-file handling can be unit-tested
/// without a native dialog.
fn import_parsed(
	db: &crate::db::Db,
	ParsedShare { author, payload }: ParsedShare,
) -> Result<serde_json::Value, String> {
	match payload {
		SharePayload::Idea {
			share_id,
			title,
			result,
			idea_type,
			created_at,
		} => {
			// Importing the same file twice should be a no-op, not a
			// duplicate library entry with a colliding share id.
			if !share_id.is_empty() {
				if let Some(existing) = db.get_idea_by_share_id(&share_id) {
					return Ok(json!({
						"cancelled": false,
						"kind": "idea",
						"duplicate": true,
						"id": existing.id,
						"title": existing.title,
						"author": author,
					}));
				}
			}
			let id = uuid::Uuid::new_v4().to_string();
			let share_id = if share_id.is_empty() {
				id.clone()
			} else {
				share_id
			};
			// imported: keeps the original timestamp but does not count
			// the row as the importer's own activity
			db.insert_idea(crate::db::NewIdea::imported(crate::db::NewIdea {
				id: &id,
				title: &title,
				idea_type,
				result: &result,
				metadata: &json!({ "imported": true }),
				creator_name: Some(&author),
				share_id: Some(&share_id),
				created_at: created_at.as_deref(),
				..Default::default()
			}))?;
			Ok(json!({
				"cancelled": false,
				"kind": "idea",
				"duplicate": false,
				"id": id,
				"title": title,
				"author": author
			}))
		}
		SharePayload::Feedback {
			target_share_id,
			target_title,
			title,
			result,
			structured_result,
			created_at,
		} => {
			let parent = db
				.get_idea_by_share_id(&target_share_id)
				.or_else(|| {
					// Fall back to a title match only when it is
					// unambiguous: titles are derived and truncated, so
					// collisions are plausible and attaching feedback to
					// the wrong idea is worse than a clean refusal.
					if target_title.is_empty() {
						return None;
					}
					let matches: Vec<_> = db
						.list_ideas()
						.unwrap_or_else(|e| {
							log::error!("library read failed during feedback import: {e}");
							vec![]
						})
						.into_iter()
						.filter(|i| i.title == target_title)
						.collect();
					if matches.len() == 1 {
						matches.into_iter().next()
					} else {
						None
					}
				})
				.ok_or_else(|| {
					"the original idea for this feedback is not in your library".to_string()
				})?;

			let title = if title.is_empty() {
				format!("Feedback: {}", parent.title)
			} else {
				title
			};
			// Same feedback file twice = no-op.
			let duplicate = db.get_idea_children(&parent.id)?.iter().any(|child| {
				child.result.as_deref() == Some(result.as_str())
					&& child.creator_name.as_deref() == Some(author.as_str())
			});
			if duplicate {
				return Ok(json!({
					"cancelled": false,
					"kind": "feedback",
					"duplicate": true,
					"parent_id": parent.id,
					"title": title,
					"author": author,
				}));
			}
			if let Some(v) = &structured_result {
				if v.to_string().len() > MAX_STRUCTURED_BYTES {
					return Err("feedback document is too large to import".into());
				}
			}
			let id = uuid::Uuid::new_v4().to_string();
			db.insert_idea(crate::db::NewIdea::imported(crate::db::NewIdea {
				id: &id,
				title: &title,
				idea_type: IdeaType::Feedback,
				result: &result,
				structured_result: structured_result.as_ref(),
				metadata: &json!({ "imported": true }),
				parent_idea_id: Some(&parent.id),
				creator_name: Some(&author),
				created_at: created_at.as_deref(),
				..Default::default()
			}))?;
			// Imported feedback arrives unread so it surfaces in the UI.
			db.set_idea_unread(&id, true)?;
			Ok(json!({
				"cancelled": false,
				"kind": "feedback",
				"id": id,
				"parent_id": parent.id,
				"title": title,
				"author": author,
			}))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{
		build_export_payload, import_parsed, parse_share_payload, ParsedShare, SharePayload,
	};
	use crate::db::Db;
	use crate::types::IdeaType;

	fn temp_db(name: &str) -> (Db, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join(format!("{name}.db"))).expect("open test db");
		(db, dir)
	}

	fn idea_share(share_id: &str, title: &str) -> ParsedShare {
		ParsedShare {
			author: "Ada".into(),
			payload: SharePayload::Idea {
				share_id: share_id.into(),
				title: title.into(),
				result: "r".into(),
				idea_type: IdeaType::Original,
				created_at: None,
			},
		}
	}

	fn feedback_share(target_share_id: &str, target_title: &str) -> ParsedShare {
		ParsedShare {
			author: "Grace".into(),
			payload: SharePayload::Feedback {
				target_share_id: target_share_id.into(),
				target_title: target_title.into(),
				title: "Feedback".into(),
				result: "notes".into(),
				structured_result: None,
				created_at: None,
			},
		}
	}

	fn local_idea(db: &Db, id: &str, title: &str, share_id: Option<&str>) {
		db.insert_idea(crate::db::NewIdea {
			id,
			title,
			idea_type: IdeaType::Original,
			result: "r",
			metadata: &serde_json::json!({}),
			share_id,
			..Default::default()
		})
		.expect("seed local idea");
	}

	#[test]
	fn import_idea_twice_is_duplicate() {
		let (db, _dir) = temp_db("dup");
		let first = import_parsed(&db, idea_share("share-1", "My Idea")).expect("first import");
		assert_eq!(first["duplicate"], false);
		assert_eq!(first["kind"], "idea");
		let second = import_parsed(&db, idea_share("share-1", "My Idea"))
			.expect("second import is a no-op, not an error");
		assert_eq!(second["duplicate"], true, "second import: {second}");
		assert_eq!(second["id"], first["id"], "points at the existing row");
		assert_eq!(db.list_ideas().expect("list").len(), 1);
	}

	#[test]
	fn import_feedback_attaches_by_share_id_then_unambiguous_title_else_refuses() {
		let (db, _dir) = temp_db("attach");
		// nothing to attach to: clean refusal
		let err = import_parsed(&db, feedback_share("ghost", "No Such"))
			.expect_err("missing target must refuse");
		assert!(err.contains("not in your library"), "unexpected: {err}");

		// share id wins even when the title is wrong
		local_idea(&db, "t1", "Real Title", Some("share-1"));
		let by_id = import_parsed(&db, feedback_share("share-1", "Wrong Title"))
			.expect("attach by share id");
		assert_eq!(by_id["parent_id"], "t1");
		assert_eq!(db.get_idea_children("t1").expect("children").len(), 1);

		// no share id match + exactly one title match: attach by title
		local_idea(&db, "t2", "Unique Title", None);
		let by_title = import_parsed(&db, feedback_share("ghost", "Unique Title"))
			.expect("attach by unambiguous title");
		assert_eq!(by_title["parent_id"], "t2");

		// no share id match + two title matches: refuse rather than guess
		local_idea(&db, "t3", "Twin", None);
		local_idea(&db, "t4", "Twin", None);
		let err = import_parsed(&db, feedback_share("ghost", "Twin"))
			.expect_err("ambiguous title match must refuse");
		assert!(err.contains("not in your library"), "unexpected: {err}");
	}

	#[test]
	fn exported_daily_intent_reimports_as_an_original_idea() {
		let (author_db, _a) = temp_db("author");
		author_db
			.create_daily_intent_idea(
				"intent",
				"My Day",
				"## Plan\nship it",
				&[],
				&serde_json::json!({}),
			)
			.expect("seed daily intent");
		let (author, payload) = super::export_share(&author_db, "intent").expect("export");
		let raw = serde_json::to_string(&build_export_payload(&author, &payload)).unwrap();

		// the strict import whitelist must accept our own export...
		let parsed = parse_share_payload(&raw).expect("own export must import");
		let (reader_db, _b) = temp_db("reader");
		let imported = import_parsed(&reader_db, parsed).expect("import");
		// ...and the copy is an ordinary idea, never the reader's intent
		let id = imported["id"].as_str().expect("id");
		let idea = reader_db.get_idea(id).unwrap().expect("stored");
		assert_eq!(idea.r#type.as_deref(), Some("original"));
		assert_eq!(idea.result.as_deref(), Some("## Plan\nship it"));
		assert_eq!(reader_db.get_daily_status().intent_idea_id, None);
	}

	#[test]
	fn import_rejects_oversized_title_and_bad_type() {
		let long_title = "x".repeat(201);
		let raw = json_string(serde_json::json!({
			"format": "brainstory-share", "version": 1, "kind": "idea",
			"idea": { "title": long_title }
		}));
		let err = parse_share_payload(&raw).expect_err("oversized title");
		assert!(err.contains("title"), "unexpected: {err}");

		let raw = json_string(serde_json::json!({
			"format": "brainstory-share", "version": 1, "kind": "idea",
			"idea": { "title": "T", "type": "daily_intent" }
		}));
		let err = parse_share_payload(&raw).expect_err("non-original idea type");
		assert!(err.contains("original ideas"), "unexpected: {err}");
	}

	fn json_string(value: serde_json::Value) -> String {
		serde_json::to_string(&value).unwrap()
	}

	fn roundtrip(payload: SharePayload, author: &str) -> ParsedShare {
		let envelope = build_export_payload(author, &payload);
		let raw = serde_json::to_string_pretty(&envelope).unwrap();
		parse_share_payload(&raw).expect("our own export must parse")
	}

	#[test]
	fn idea_roundtrip_preserves_fields() {
		let parsed = roundtrip(
			SharePayload::Idea {
				share_id: "share-123".into(),
				title: "My Idea".into(),
				result: "## Heading\nBody".into(),
				idea_type: IdeaType::Original,
				created_at: Some("2026-09-15T10:30:00".into()),
			},
			"Ada",
		);
		assert_eq!(parsed.author, "Ada");
		match parsed.payload {
			SharePayload::Idea {
				share_id,
				title,
				result,
				idea_type,
				created_at,
			} => {
				assert_eq!(share_id, "share-123");
				assert_eq!(title, "My Idea");
				assert_eq!(result, "## Heading\nBody");
				assert_eq!(idea_type, IdeaType::Original);
				assert_eq!(created_at.as_deref(), Some("2026-09-15T10:30:00"));
			}
			_ => panic!("wrong payload kind"),
		}
	}

	#[test]
	fn feedback_roundtrip_preserves_fields_including_structured() {
		let structured = serde_json::json!({ "sections": [ { "heading": "# H", "body": "b" } ] });
		let parsed = roundtrip(
			SharePayload::Feedback {
				target_share_id: "target-1".into(),
				target_title: "Target".into(),
				title: "Feedback: Target".into(),
				result: "notes".into(),
				structured_result: Some(structured.clone()),
				created_at: None,
			},
			"Grace",
		);
		assert_eq!(parsed.author, "Grace");
		match parsed.payload {
			SharePayload::Feedback {
				target_share_id,
				structured_result,
				created_at,
				..
			} => {
				assert_eq!(target_share_id, "target-1");
				assert_eq!(structured_result.as_ref(), Some(&structured));
				assert_eq!(created_at, None);
			}
			_ => panic!("wrong payload kind"),
		}
	}

	#[test]
	fn rejects_bad_json_and_wrong_format() {
		assert!(parse_share_payload("not json").is_err());
		assert!(
			parse_share_payload(r#"{"format": "other", "version": 1, "kind": "idea"}"#).is_err()
		);
	}

	#[test]
	fn rejects_missing_and_future_versions() {
		assert!(parse_share_payload(r#"{"format": "brainstory-share", "kind": "idea"}"#).is_err());
		let future = r#"{"format": "brainstory-share", "version": 99, "kind": "idea"}"#;
		let err = parse_share_payload(future).expect_err("future version must fail");
		assert!(err.contains("newer"), "unexpected error: {err}");
	}

	#[test]
	fn rejects_unknown_kind() {
		let raw = r#"{"format": "brainstory-share", "version": 1, "kind": "meme"}"#;
		let err = parse_share_payload(raw).expect_err("unknown kind must fail");
		assert!(err.contains("meme"));
	}

	#[test]
	fn missing_optional_fields_fall_back_to_defaults() {
		let raw = r#"{"format": "brainstory-share", "version": 1, "kind": "idea", "idea": {"title": 42}}"#;
		let parsed = parse_share_payload(raw).expect("structurally valid file");
		assert_eq!(parsed.author, "Anonymous");
		match parsed.payload {
			SharePayload::Idea {
				share_id,
				idea_type,
				..
			} => {
				assert_eq!(share_id, "");
				assert_eq!(idea_type, IdeaType::Original);
			}
			_ => panic!("wrong payload kind"),
		}
	}

	#[test]
	fn malformed_created_at_is_dropped_not_fatal() {
		let raw = r#"{"format": "brainstory-share", "version": 1, "kind": "idea",
			"idea": {"created_at": "yesterday"}}"#;
		let parsed = parse_share_payload(raw).expect("parses fine");
		match parsed.payload {
			SharePayload::Idea { created_at, .. } => assert_eq!(created_at, None),
			_ => panic!("wrong payload kind"),
		}
	}
}
