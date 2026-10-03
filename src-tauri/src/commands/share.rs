use serde_json::json;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::reactions::{is_valid_reaction, SharedSectionReaction};
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
/// Most section reactions a feedback file may carry: every one of the 8
/// emojis on ~60 sections. More means a hostile file, not a keen reader.
const MAX_IMPORTED_REACTIONS: usize = 500;

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
		/// the feedback author's own reactions on the target idea's
		/// sections (optional in the file; older files have none)
		reactions: Vec<SharedSectionReaction>,
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
			reactions,
		} => {
			root["kind"] = json!("feedback");
			root["feedback"] = json!({
				"target_share_id": target_share_id,
				"target_title": target_title,
				"title": title,
				"result": result,
				"structured_result": structured_result,
				"created_at": created_at,
				"reactions": reactions,
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
				// only feedback-contract documents are stored; a malformed
				// one degrades to none, keeping the readable feedback
				structured_result: sanitize_structured_feedback(&feedback["structured_result"]),
				created_at: parse_created_at(feedback["created_at"].as_str()),
				reactions: parse_reactions(&feedback["reactions"])?,
			}
		}
		other => return Err(format!("unknown share kind: {other}")),
	};
	Ok(ParsedShare { author, payload })
}

/// Validate an imported structured feedback document against the
/// feedback contract (an object with a `feedback_items` array of
/// members; the model-generated twin is `extract_feedback_json` in
/// commands/ai.rs). A document whose container is unusable - not an
/// object, no `feedback_items` key, or a non-array value - is degraded
/// to none, exactly like unparseable model output, so the readable
/// feedback idea still imports. Unusable members are skipped instead so
/// the valid ones beside them survive; survivors keep their original
/// order (comment reactions are keyed by index into the stored array, so
/// a skipped member must never renumber them). Other top-level keys of
/// a valid document pass through unchanged.
fn sanitize_structured_feedback(value: &serde_json::Value) -> Option<serde_json::Value> {
	let mut doc = value.as_object()?.clone();
	let items = doc.get("feedback_items")?.as_array()?.clone();
	doc.insert(
		"feedback_items".into(),
		serde_json::Value::Array(
			items
				.iter()
				.filter(|item| is_usable_feedback_item(item))
				.cloned()
				.collect(),
		),
	);
	Some(serde_json::Value::Object(doc))
}

/// A structured feedback member is usable when it is an object whose
/// `feedback_text` is a non-empty string (the visible comment) and whose
/// `oid_heading_text`, when present, is a string: a non-string heading
/// can never be placed on a section and crashes the frontend heading
/// parser at display time. An absent or null heading is fine
/// (aggregation logs and skips those comments), matching the model
/// pipeline's historical tolerance.
pub(crate) fn is_usable_feedback_item(item: &serde_json::Value) -> bool {
	let Some(obj) = item.as_object() else {
		return false;
	};
	let text = obj.get("feedback_text").and_then(|t| t.as_str());
	match text.map(str::trim) {
		Some(t) if !t.is_empty() => {}
		_ => return false,
	}
	match obj.get("oid_heading_text") {
		None | Some(serde_json::Value::Null) => true,
		Some(heading) => heading.is_string(),
	}
}

/// The optional section reactions of a feedback file. Like the other
/// optional fields, a missing or wrongly-typed `reactions` falls back to
/// none, and an entry that is not a whitelisted emoji on a non-negative
/// integer section index is skipped rather than failing the whole file
/// (so a newer build's extra emojis cost only those reactions, never the
/// feedback itself). An oversized list is a hostile file and refused,
/// like an oversized title.
fn parse_reactions(value: &serde_json::Value) -> Result<Vec<SharedSectionReaction>, String> {
	let Some(entries) = value.as_array() else {
		return Ok(Vec::new());
	};
	if entries.len() > MAX_IMPORTED_REACTIONS {
		return Err(format!(
			"share file carries too many reactions (limit {MAX_IMPORTED_REACTIONS})"
		));
	}
	let mut reactions: Vec<SharedSectionReaction> = Vec::new();
	for entry in entries {
		let section_index = entry["section_index"].as_i64().filter(|i| *i >= 0);
		let emoji = entry["emoji"].as_str().filter(|e| is_valid_reaction(e));
		let (Some(section_index), Some(emoji)) = (section_index, emoji) else {
			log::warn!("skipping an invalid reaction in a share file");
			continue;
		};
		let reaction = SharedSectionReaction {
			section_index,
			emoji: emoji.to_string(),
		};
		if !reactions.contains(&reaction) {
			reactions.push(reaction);
		}
	}
	Ok(reactions)
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
		let (target_share_id, target_title, reactions) = match &idea.parent_idea {
			Some(parent) => (
				db.get_share_id(&parent.id)
					.unwrap_or_else(|| parent.id.clone()),
				parent.title.clone(),
				// my own reactions on the idea this feedback is about
				// (never ones other people's imported feedback carried)
				db.my_section_reactions(&parent.id)?,
			),
			None => (String::new(), String::new(), Vec::new()),
		};
		SharePayload::Feedback {
			target_share_id,
			target_title,
			title: idea.title.clone(),
			result: idea.result.clone().unwrap_or_default(),
			structured_result: idea.structured_result.clone(),
			created_at: Some(idea.created_at.clone()),
			reactions,
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
			reactions,
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
			// Same feedback file twice = no-op (its reactions included:
			// they were stored with the first import).
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
				// stored on the parent's sections, attributed to this
				// feedback (deleting it deletes them)
				parent_section_reactions: &reactions,
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
	use crate::reactions::SharedSectionReaction;
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
				reactions: vec![],
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

	#[test]
	fn import_caps_the_result_document_at_one_megabyte() {
		let file = |result: String| {
			json_string(serde_json::json!({
				"format": "brainstory-share", "version": 1, "kind": "idea",
				"idea": { "title": "T", "result": result }
			}))
		};
		let cap = super::MAX_RESULT_BYTES;
		assert_eq!(cap, 1024 * 1024);
		parse_share_payload(&file("x".repeat(cap))).expect("exactly at the cap is fine");
		let err = parse_share_payload(&file("x".repeat(cap + 1))).expect_err("over the cap");
		assert!(err.contains("result is too large"), "unexpected: {err}");
		// feedback documents go through the same bound
		let raw = json_string(serde_json::json!({
			"format": "brainstory-share", "version": 1, "kind": "feedback",
			"feedback": { "title": "F", "result": "x".repeat(cap + 1) }
		}));
		assert!(parse_share_payload(&raw).is_err());
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
		// a conforming feedback document survives byte-for-byte
		let structured = serde_json::json!({
			"feedback_items": [
				{ "oid_heading_text": "1## H", "matched_spans": ["span"], "feedback_text": "b" }
			]
		});
		let parsed = roundtrip(
			SharePayload::Feedback {
				target_share_id: "target-1".into(),
				target_title: "Target".into(),
				title: "Feedback: Target".into(),
				result: "notes".into(),
				structured_result: Some(structured.clone()),
				created_at: None,
				reactions: vec![
					SharedSectionReaction {
						section_index: 1,
						emoji: "⚠️".into(),
					},
					SharedSectionReaction {
						section_index: 3,
						emoji: "🚀".into(),
					},
				],
			},
			"Grace",
		);
		assert_eq!(parsed.author, "Grace");
		match parsed.payload {
			SharePayload::Feedback {
				target_share_id,
				structured_result,
				created_at,
				reactions,
				..
			} => {
				assert_eq!(target_share_id, "target-1");
				assert_eq!(structured_result.as_ref(), Some(&structured));
				assert_eq!(created_at, None);
				let emojis: Vec<(i64, &str)> = reactions
					.iter()
					.map(|r| (r.section_index, r.emoji.as_str()))
					.collect();
				assert_eq!(emojis, vec![(1, "⚠️"), (3, "🚀")]);
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

	fn reaction(section_index: i64, emoji: &str) -> SharedSectionReaction {
		SharedSectionReaction {
			section_index,
			emoji: emoji.into(),
		}
	}

	fn feedback_file(reactions: serde_json::Value) -> String {
		json_string(serde_json::json!({
			"format": "brainstory-share", "version": 1, "kind": "feedback", "author": "Grace",
			"feedback": {
				"target_share_id": "share-1", "title": "F", "result": "notes",
				"reactions": reactions,
			}
		}))
	}

	fn parsed_reactions(raw: &str) -> Vec<SharedSectionReaction> {
		match parse_share_payload(raw).expect("parses").payload {
			SharePayload::Feedback { reactions, .. } => reactions,
			_ => panic!("wrong payload kind"),
		}
	}

	/// Ada shares an idea, Grace reacts to its sections and sends feedback
	/// back: Ada sees Grace's reactions on her own idea.
	#[test]
	fn feedback_carries_my_section_reactions_back_to_the_author() {
		let (ada, _a) = temp_db("ada");
		ada.set_setting(crate::keys::setting::USER_NAME, "Ada")
			.unwrap();
		ada.insert_idea(crate::db::NewIdea {
			id: "idea",
			title: "Plan",
			idea_type: IdeaType::Original,
			result: "## One\na\n\n## Two\nb",
			metadata: &serde_json::json!({}),
			..Default::default()
		})
		.unwrap();
		// Ada's own reaction on her idea never travels with an idea export
		ada.toggle_section_reaction("idea", 2, "🚀").unwrap();
		let (author, payload) = super::export_share(&ada, "idea").expect("export idea");
		let envelope = build_export_payload(&author, &payload);
		assert!(envelope["idea"].get("reactions").is_none(), "{envelope}");
		assert!(envelope.get("reactions").is_none(), "{envelope}");

		let (grace, _g) = temp_db("grace");
		grace
			.set_setting(crate::keys::setting::USER_NAME, "Grace")
			.unwrap();
		let raw = serde_json::to_string(&envelope).unwrap();
		let imported = import_parsed(&grace, parse_share_payload(&raw).unwrap()).unwrap();
		let idea_copy = imported["id"].as_str().unwrap().to_string();
		grace.toggle_section_reaction(&idea_copy, 1, "👍").unwrap();
		grace.toggle_section_reaction(&idea_copy, 2, "⚠️").unwrap();
		grace.toggle_section_reaction(&idea_copy, 2, "❓").unwrap();
		grace.toggle_section_reaction(&idea_copy, 2, "❓").unwrap(); // off again
		grace
			.insert_idea(crate::db::NewIdea {
				id: "grace-fb",
				title: "Feedback: Plan",
				idea_type: IdeaType::Feedback,
				result: "looks good",
				metadata: &serde_json::json!({}),
				parent_idea_id: Some(&idea_copy),
				..Default::default()
			})
			.unwrap();
		let (author, payload) = super::export_share(&grace, "grace-fb").expect("export feedback");
		assert_eq!(author, "Grace");
		let envelope = build_export_payload(&author, &payload);
		assert_eq!(
			envelope["feedback"]["reactions"],
			serde_json::json!([
				{ "section_index": 1, "emoji": "👍" },
				{ "section_index": 2, "emoji": "⚠️" },
			]),
			"exactly my current reactions on the parent idea"
		);

		let raw = serde_json::to_string_pretty(&envelope).unwrap();
		let result = import_parsed(&ada, parse_share_payload(&raw).unwrap()).expect("import");
		assert_eq!(result["parent_id"], "idea");
		let feedback_id = result["id"].as_str().unwrap().to_string();
		let theirs = |section_index: i64, emoji: &str| crate::reactions::SectionReaction {
			section_index,
			emoji: emoji.into(),
			mine: false,
			from: Some("Grace".into()),
		};
		let mine = crate::reactions::SectionReaction {
			section_index: 2,
			emoji: "🚀".into(),
			mine: true,
			from: None,
		};
		assert_eq!(
			ada.get_reactions("idea").unwrap().sections,
			vec![theirs(1, "👍"), mine.clone(), theirs(2, "⚠️")]
		);

		// importing the same file again is a duplicate: no second copy
		let again = import_parsed(&ada, parse_share_payload(&raw).unwrap()).unwrap();
		assert_eq!(again["duplicate"], true);
		assert_eq!(ada.get_reactions("idea").unwrap().sections.len(), 3);

		// Ada's feedback export to someone else carries only her own
		// reactions, never the ones Grace's file brought in
		assert_eq!(
			ada.my_section_reactions("idea").unwrap(),
			vec![reaction(2, "🚀")]
		);

		// deleting Grace's feedback takes her reactions with it
		assert!(ada.delete_idea(&feedback_id).unwrap());
		assert_eq!(ada.get_reactions("idea").unwrap().sections, vec![mine]);
	}

	#[test]
	fn feedback_without_reactions_exports_an_empty_list() {
		let (db, _dir) = temp_db("plain");
		local_idea(&db, "idea", "Plan", Some("share-1"));
		db.insert_idea(crate::db::NewIdea {
			id: "fb",
			title: "F",
			idea_type: IdeaType::Feedback,
			result: "notes",
			metadata: &serde_json::json!({}),
			parent_idea_id: Some("idea"),
			..Default::default()
		})
		.unwrap();
		let (author, payload) = super::export_share(&db, "fb").unwrap();
		let envelope = build_export_payload(&author, &payload);
		assert_eq!(envelope["feedback"]["reactions"], serde_json::json!([]));
		assert_eq!(
			envelope["version"], 1,
			"the share format version is unchanged"
		);
	}

	#[test]
	fn older_feedback_files_without_reactions_import_as_before() {
		let raw = json_string(serde_json::json!({
			"format": "brainstory-share", "version": 1, "kind": "feedback", "author": "Grace",
			"feedback": { "target_share_id": "share-1", "title": "F", "result": "notes" }
		}));
		assert!(parsed_reactions(&raw).is_empty());
		let (db, _dir) = temp_db("old");
		local_idea(&db, "idea", "Plan", Some("share-1"));
		let result = import_parsed(&db, parse_share_payload(&raw).unwrap()).expect("imports");
		assert_eq!(result["parent_id"], "idea");
		assert_eq!(db.get_idea_children("idea").unwrap().len(), 1);
		assert!(db.get_reactions("idea").unwrap().sections.is_empty());
		// a null or wrongly-typed field falls back to none, like other
		// optional fields
		for value in [
			serde_json::Value::Null,
			serde_json::json!("👍"),
			serde_json::json!({}),
		] {
			assert!(parsed_reactions(&feedback_file(value)).is_empty());
		}
	}

	#[test]
	fn invalid_reactions_are_skipped_not_fatal() {
		let raw = feedback_file(serde_json::json!([
			{ "section_index": 1, "emoji": "👍" },
			{ "section_index": 1, "emoji": "❤️" },
			{ "section_index": 1, "emoji": "\u{26A0}" },
			{ "section_index": -1, "emoji": "👍" },
			{ "section_index": 1.5, "emoji": "👍" },
			{ "section_index": "2", "emoji": "👍" },
			{ "emoji": "👍" },
			{ "section_index": 2 },
			"👍",
			{ "section_index": 2, "emoji": "📚" },
			{ "section_index": 1, "emoji": "👍" },
		]));
		assert_eq!(
			parsed_reactions(&raw),
			vec![reaction(1, "👍"), reaction(2, "📚")],
			"valid entries kept once, everything else dropped"
		);
		let (db, _dir) = temp_db("invalid");
		local_idea(&db, "idea", "Plan", Some("share-1"));
		import_parsed(&db, parse_share_payload(&raw).unwrap()).expect("still imports");
		let stored: Vec<(i64, String)> = db
			.get_reactions("idea")
			.unwrap()
			.sections
			.into_iter()
			.map(|r| (r.section_index, r.emoji))
			.collect();
		assert_eq!(stored, vec![(1, "👍".to_string()), (2, "📚".to_string())]);
	}

	#[test]
	fn too_many_reactions_reject_the_file() {
		let cap = super::MAX_IMPORTED_REACTIONS;
		let entries = |n: usize| {
			serde_json::Value::Array(
				(0..n)
					.map(|i| serde_json::json!({ "section_index": i, "emoji": "👍" }))
					.collect(),
			)
		};
		assert_eq!(parsed_reactions(&feedback_file(entries(cap))).len(), cap);
		let err = parse_share_payload(&feedback_file(entries(cap + 1))).expect_err("over the cap");
		assert!(err.contains("too many reactions"), "unexpected: {err}");
	}

	fn structured_feedback_file(structured: serde_json::Value) -> String {
		json_string(serde_json::json!({
			"format": "brainstory-share", "version": 1, "kind": "feedback", "author": "Grace",
			"feedback": {
				"target_share_id": "share-1", "title": "F", "result": "notes",
				"structured_result": structured,
			}
		}))
	}

	fn parsed_structured(raw: &str) -> Option<serde_json::Value> {
		match parse_share_payload(raw).expect("parses").payload {
			SharePayload::Feedback {
				structured_result, ..
			} => structured_result,
			_ => panic!("wrong payload kind"),
		}
	}

	/// F01: a malformed structured document must not take the readable
	/// feedback down with it. An unusable container (non-array
	/// feedback_items, a document without the key, a non-object
	/// document) degrades the document to none - exactly what
	/// extract_feedback_json does with unparseable model output - and
	/// the feedback idea itself still imports with its result intact.
	#[test]
	fn malformed_structured_feedback_container_degrades_to_no_document() {
		for broken in [
			serde_json::json!({ "feedback_items": {} }),
			serde_json::json!({ "feedback_items": "nope" }),
			serde_json::json!({ "sections": [{ "heading": "# H" }] }),
			serde_json::json!([1, 2]),
			serde_json::Value::Null,
		] {
			let raw = structured_feedback_file(broken.clone());
			assert_eq!(
				parsed_structured(&raw),
				None,
				"container {broken} must degrade to no document"
			);
		}
		// the feedback itself still lands in the library, readable
		let (db, _dir) = temp_db("broken-container");
		local_idea(&db, "idea", "Plan", Some("share-1"));
		let raw = structured_feedback_file(serde_json::json!({ "feedback_items": {} }));
		let imported = import_parsed(&db, parse_share_payload(&raw).unwrap()).expect("imports");
		assert_eq!(imported["parent_id"], "idea");
		let children = db.get_idea_children("idea").unwrap();
		assert_eq!(children.len(), 1, "the malformed child is not hidden");
		assert_eq!(children[0].result.as_deref(), Some("notes"));
		assert_eq!(children[0].structured_result, None);
	}

	/// Unusable MEMBERS are skipped, never the whole list, and survivors
	/// are stored in their original order (comment reactions are keyed
	/// by index into the stored array, so a skipped member must not
	/// renumber them). A member needs an object shape, non-empty string
	/// feedback_text, and a string (or absent/null) oid_heading_text - a
	/// non-string heading can never be placed on a section.
	#[test]
	fn unusable_structured_feedback_members_are_skipped_not_the_list() {
		let structured = serde_json::json!({
			"feedback_items": [
				{ "oid_heading_text": "1## A", "matched_spans": [], "feedback_text": "keep" },
				null,
				42,
				"note",
				[],
				{ "oid_heading_text": "2## B", "feedback_text": "" },
				{ "oid_heading_text": "2## B", "feedback_text": "  " },
				{ "oid_heading_text": 3, "feedback_text": "numbered heading" },
				{ "oid_heading_text": { "a": 1 }, "feedback_text": "object heading" },
				{ "feedback_text": "keep without heading" },
				{ "oid_heading_text": null, "feedback_text": "keep with null heading" },
				{ "oid_heading_text": "3## C", "matched_spans": "not an array", "feedback_text": "keep" },
			]
		});
		let expected = serde_json::json!({
			"feedback_items": [
				{ "oid_heading_text": "1## A", "matched_spans": [], "feedback_text": "keep" },
				{ "feedback_text": "keep without heading" },
				{ "oid_heading_text": null, "feedback_text": "keep with null heading" },
				{ "oid_heading_text": "3## C", "matched_spans": "not an array", "feedback_text": "keep" },
			]
		});
		let raw = structured_feedback_file(structured);
		assert_eq!(parsed_structured(&raw), Some(expected));

		// and the filtered document is what lands in the database
		let (db, _dir) = temp_db("filtered-members");
		local_idea(&db, "idea", "Plan", Some("share-1"));
		import_parsed(&db, parse_share_payload(&raw).unwrap()).expect("imports");
		let children = db.get_idea_children("idea").unwrap();
		assert_eq!(children.len(), 1);
		assert_eq!(
			children[0].structured_result.as_ref().unwrap()["feedback_items"]
				.as_array()
				.unwrap()
				.len(),
			4,
			"valid members survive beside invalid ones"
		);
	}
}
