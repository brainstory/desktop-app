use tauri::State;

use crate::reactions::IdeaReactions;
use crate::AppState;

/// Reactions shown on an idea's page: its section reactions (`mine` for
/// the user's own, `from` naming the imported feedback's author) and the
/// author's reactions on the comments of its feedback children.
#[tauri::command]
pub async fn get_reactions(
	state: State<'_, AppState>,
	idea_id: String,
) -> Result<IdeaReactions, String> {
	state.db.get_reactions(&idea_id)
}

/// Toggle the user's own reaction on one section of an idea; returns true
/// when the reaction is now on.
#[tauri::command]
pub async fn toggle_section_reaction(
	state: State<'_, AppState>,
	idea_id: String,
	section_index: i64,
	emoji: String,
) -> Result<bool, String> {
	state
		.db
		.toggle_section_reaction(&idea_id, section_index, &emoji)
}

/// Toggle the idea author's reaction on one comment (feedback item) of a
/// feedback idea; returns true when the reaction is now on. Comment
/// reactions stay local - no share file carries them.
#[tauri::command]
pub async fn toggle_comment_reaction(
	state: State<'_, AppState>,
	feedback_idea_id: String,
	item_index: i64,
	emoji: String,
) -> Result<bool, String> {
	state
		.db
		.toggle_comment_reaction(&feedback_idea_id, item_index, &emoji)
}
