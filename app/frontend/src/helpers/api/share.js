import { invoke } from "@tauri-apps/api/core";

/**
 * Export an idea (or feedback) as a shareable JSON file. Opens a save
 * dialog; the user sends the file to the other person however they like.
 */
export async function exportIdeaApi(ideaId) {
	return invoke("export_idea", { ideaId });
}

/** Import a shared idea or feedback JSON file. Opens a file dialog. */
export async function importShareApi() {
	return invoke("import_share");
}
