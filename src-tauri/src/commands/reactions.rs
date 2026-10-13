use tauri::Manager;

use crate::commands::run_blocking;
use crate::reactions::IdeaReactions;
use crate::AppState;

/// Reactions shown on an idea's page: its section reactions (`mine` for
/// the user's own, `from` naming the imported feedback's author) and the
/// author's reactions on the comments of its feedback children.
#[tauri::command]
pub async fn get_reactions(
	app: tauri::AppHandle,
	idea_id: String,
) -> Result<IdeaReactions, String> {
	run_blocking("get reactions", move || {
		let state = app.state::<AppState>();
		state.db.get_reactions(&idea_id)
	})
	.await
}

/// Toggle the user's own reaction on one section of an idea; returns true
/// when the reaction is now on.
#[tauri::command]
pub async fn toggle_section_reaction(
	app: tauri::AppHandle,
	idea_id: String,
	section_index: i64,
	emoji: String,
) -> Result<bool, String> {
	run_blocking("toggle section reaction", move || {
		let state = app.state::<AppState>();
		state
			.db
			.toggle_section_reaction(&idea_id, section_index, &emoji)
	})
	.await
}

/// Toggle the idea author's reaction on one comment (feedback item) of a
/// feedback idea; returns true when the reaction is now on. Comment
/// reactions stay local - no share file carries them.
#[tauri::command]
pub async fn toggle_comment_reaction(
	app: tauri::AppHandle,
	feedback_idea_id: String,
	item_index: i64,
	emoji: String,
) -> Result<bool, String> {
	run_blocking("toggle comment reaction", move || {
		let state = app.state::<AppState>();
		state
			.db
			.toggle_comment_reaction(&feedback_idea_id, item_index, &emoji)
	})
	.await
}
