use serde_json::json;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

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
		idea_type: String,
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
			SharePayload::Idea {
				share_id: idea["share_id"].as_str().unwrap_or("").to_string(),
				title: idea["title"]
					.as_str()
					.unwrap_or("Imported idea")
					.to_string(),
				result: idea["result"].as_str().unwrap_or("").to_string(),
				idea_type: idea["type"].as_str().unwrap_or("original").to_string(),
				created_at: parse_created_at(idea["created_at"].as_str()),
			}
		}
		"feedback" => {
			let feedback = &root["feedback"];
			SharePayload::Feedback {
				target_share_id: feedback["target_share_id"].as_str().unwrap_or("").to_string(),
				target_title: feedback["target_title"].as_str().unwrap_or("").to_string(),
				title: feedback["title"].as_str().unwrap_or("").to_string(),
				result: feedback["result"].as_str().unwrap_or("").to_string(),
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
	let idea = state
		.db
		.get_idea(&idea_id)?
		.ok_or_else(|| format!("idea {idea_id} not found"))?;

	let idea_type = idea.r#type.clone().unwrap_or_else(|| "original".into());
	let own_name = state.db.get_setting("user_name").filter(|s| !s.is_empty());
	let author = idea
		.creator_name
		.clone()
		.or(own_name)
		.unwrap_or_else(|| "Anonymous".into());

	let payload = if idea_type == "feedback" {
		let (target_share_id, target_title) = match &idea.parent_idea {
			Some(parent) => (
				state
					.db
					.get_share_id(&parent.id)
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
		let share_id = state
			.db
			.get_share_id(&idea.id)
			.unwrap_or_else(|| idea.id.clone());
		SharePayload::Idea {
			share_id,
			title: idea.title.clone(),
			result: idea.result.clone().unwrap_or_default(),
			idea_type: idea_type.clone(),
			created_at: Some(idea.created_at.clone()),
		}
	};

	let envelope = build_export_payload(&author, &payload);
	let default_name = format!(
		"brainstory-{}-{}.json",
		idea_type,
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
	let path = receiver
		.await
		.map_err(|e| e.to_string())?;
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
	let path = receiver
		.await
		.map_err(|e| e.to_string())?;
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
	let ParsedShare { author, payload } = parse_share_payload(&raw)?;

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
				if let Some(existing) = state.db.get_idea_by_share_id(&share_id) {
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
			state.db.insert_idea(
				&id,
				&title,
				&idea_type,
				&result,
				None,
				&[],
				&json!({ "imported": true }),
				None,
				None,
				Some(&author),
				None,
				Some(&share_id),
				created_at.as_deref(),
			)?;
			Ok(
				json!({ "cancelled": false, "kind": "idea", "id": id, "title": title, "author": author }),
			)
		}
		SharePayload::Feedback {
			target_share_id,
			target_title,
			title,
			result,
			structured_result,
			created_at,
		} => {
			let parent = state
				.db
				.get_idea_by_share_id(&target_share_id)
				.or_else(|| {
					// Fall back to a title match only when it is
					// unambiguous: titles are derived and truncated, so
					// collisions are plausible and attaching feedback to
					// the wrong idea is worse than a clean refusal.
					if target_title.is_empty() {
						return None;
					}
					let matches: Vec<_> = state
						.db
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
			let duplicate = state
				.db
				.get_idea_children(&parent.id)?
				.iter()
				.any(|child| {
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
			state.db.insert_idea(
				&id,
				&title,
				"feedback",
				&result,
				structured_result.as_ref(),
				&[],
				&json!({ "imported": true }),
				Some(&parent.id),
				None,
				Some(&author),
				None,
				None,
				created_at.as_deref(),
			)?;
			// Imported feedback arrives unread so it surfaces in the UI.
			state.db.set_idea_unread(&id, true)?;
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
	use super::{build_export_payload, parse_share_payload, SharePayload, ParsedShare};

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
				idea_type: "original".into(),
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
				assert_eq!(idea_type, "original");
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
		assert!(parse_share_payload(r#"{"format": "other", "version": 1, "kind": "idea"}"#).is_err());
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
				share_id, idea_type, ..
			} => {
				assert_eq!(share_id, "");
				assert_eq!(idea_type, "original");
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
