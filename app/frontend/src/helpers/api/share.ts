import { invoke } from "@tauri-apps/api/core";

export interface ExportIdeaResult {
	cancelled: boolean;
	path?: string;
	kind?: string;
}

export interface ImportShareResult {
	cancelled: boolean;
	kind?: string;
	duplicate?: boolean;
	id?: string;
	parent_id?: string;
	title?: string;
	author?: string;
}

/**
 * Export an idea (or feedback) as a shareable JSON file. Opens a save
 * dialog; the user sends the file to the other person however they like.
 */
export function exportIdeaApi(ideaId: string): Promise<ExportIdeaResult> {
	return invoke("export_idea", { ideaId });
}

/** Import a shared idea or feedback JSON file. Opens a file dialog. */
export function importShareApi(): Promise<ImportShareResult> {
	return invoke("import_share");
}
