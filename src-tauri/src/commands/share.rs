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
const MAX_SHARE_FILE_BYTES: u64 = 10 * 1024 * 1024;/// Largest structured feedback document we will store.
const MAX_STRUCTURED_BYTES: usize = 1024 * 1024;

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

	let mut payload = json!({
		"format": "brainstory-share",
		"version": 1,
		"exported_at": chrono::Utc::now().to_rfc3339(),
		"author": author,
	});

	if idea_type == "feedback" {
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
		payload["kind"] = json!("feedback");
		payload["feedback"] = json!({
			"target_share_id": target_share_id,
			"target_title": target_title,
			"title": idea.title,
			"result": idea.result,
			"structured_result": idea.structured_result,
			"created_at": idea.created_at,
		});
	} else {
		let share_id = state
			.db
			.get_share_id(&idea.id)
			.unwrap_or_else(|| idea.id.clone());
		payload["kind"] = json!("idea");
		payload["idea"] = json!({
			"share_id": share_id,
			"title": idea.title,
			"result": idea.result,
			"type": idea.r#type,
			"created_at": idea.created_at,
		});
	}

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

	std::fs::write(&path, serde_json::to_string_pretty(&payload).unwrap())
		.map_err(|e| e.to_string())?;

	Ok(json!({ "cancelled": false, "path": path.to_string_lossy(), "kind": payload["kind"] }))
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
	let payload: serde_json::Value =
		serde_json::from_str(&raw).map_err(|e| format!("not a valid share file: {e}"))?;
	if payload["format"].as_str() != Some("brainstory-share") {
		return Err("not a brainstory share file".into());
	}
	match payload["version"].as_i64() {
		Some(1) => {}
		Some(v) => {
			return Err(format!(
				"share file version {v} is newer than this app supports - update Brainstory and try again"
			))
		}
		None => return Err("share file is missing its version".into()),
	}
	let author = payload["author"]
		.as_str()
		.filter(|s| !s.is_empty())
		.unwrap_or("Anonymous");
	let kind = payload["kind"]
		.as_str()
		.ok_or_else(|| "share file is missing its kind".to_string())?;
	// Prefer the original creation date; fall back to now if it's malformed.
	let parse_created_at = |value: Option<&str>| -> Option<String> {
		let value = value?;
		chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
			.ok()
			.map(|_| value.to_string())
	};

	match kind {
		"idea" => {
			let idea = &payload["idea"];
			let title = idea["title"]
				.as_str()
				.unwrap_or("Imported idea")
				.to_string();
			let share_id = idea["share_id"].as_str().unwrap_or("").to_string();
			// Importing the same file twice should be a no-op, not a duplicate
			// library entry with a colliding share id.
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
			let result = idea["result"].as_str().unwrap_or("").to_string();
			let idea_type = idea["type"].as_str().unwrap_or("original").to_string();
			let id = uuid::Uuid::new_v4().to_string();
			let share_id = if share_id.is_empty() {
				id.clone()
			} else {
				share_id
			};
			let created_at = parse_created_at(idea["created_at"].as_str());
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
				Some(author),
				None,
				Some(&share_id),
				created_at.as_deref(),
			)?;
			Ok(
				json!({ "cancelled": false, "kind": "idea", "id": id, "title": title, "author": author }),
			)
		}
		"feedback" => {
			let feedback = &payload["feedback"];
			let target_share_id = feedback["target_share_id"].as_str().unwrap_or("");
			let parent = state
				.db
				.get_idea_by_share_id(target_share_id)
				.or_else(|| {
					// Fall back to a title match only when it is
					// unambiguous: titles are derived and truncated, so
					// collisions are plausible and attaching feedback to
					// the wrong idea is worse than a clean refusal.
					let title = feedback["target_title"].as_str()?;
					let matches: Vec<_> = state
						.db
						.list_ideas()
						.into_iter()
						.filter(|i| i.title == title)
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

			let title = feedback["title"]
				.as_str()
				.map(|s| s.to_string())
				.filter(|s| !s.is_empty())
				.unwrap_or_else(|| format!("Feedback: {}", parent.title));
			let result = feedback["result"].as_str().unwrap_or("").to_string();
			// Same feedback file twice = no-op.
			let duplicate = state.db.get_idea_children(&parent.id).iter().any(|child| {
				child.result.as_deref() == Some(result.as_str())
					&& child.creator_name.as_deref() == Some(author)
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
			let structured = feedback["structured_result"]
				.as_object()
				.map(|_| feedback["structured_result"].clone());
			if let Some(v) = &structured {
				if v.to_string().len() > MAX_STRUCTURED_BYTES {
					return Err("feedback document is too large to import".into());
				}
			}
			let id = uuid::Uuid::new_v4().to_string();
			let created_at = parse_created_at(feedback["created_at"].as_str());
			state.db.insert_idea(
				&id,
				&title,
				"feedback",
				&result,
				structured.as_ref(),
				&[],
				&json!({ "imported": true }),
				Some(&parent.id),
				None,
				Some(author),
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
		other => Err(format!("unknown share kind: {other}")),
	}
}
