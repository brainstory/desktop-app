import { safeParse } from "valibot";
import { invokeCommand } from "@src/tauri/invoke";
import { ImportShareResultSchema } from "./schemas";

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
	return invokeCommand("exportIdea", { ideaId });
}

/** Export an idea as a Markdown file (optionally with its transcript)
 * through the save dialog. */
export function exportMarkdownApi(
	ideaId: string,
	includeTranscript: boolean
): Promise<ExportIdeaResult> {
	return invokeCommand("exportMarkdown", { ideaId, includeTranscript });
}

/** Shorten a leading home directory to ~ for display. */
export function shortenHomePath(path: string): string {
	return path.replace(/^(?:\/Users|C:\\Users)[/\\]?[^/\\]+/, "~");
}

/** Import a shared idea or feedback JSON file. Opens a file dialog.
 * The result's string fields pass through from the imported file, so it
 * is validated on read; a malformed result reads as "cancelled" (no-op)
 * instead of surfacing garbage in the snackbar. */
export async function importShareApi(): Promise<ImportShareResult> {
	const response = await invokeCommand("importShare");
	const parsed = safeParse(ImportShareResultSchema, response);
	if (!parsed.success) {
		console.warn("share import returned a malformed result; treating it as cancelled", {
			issues: parsed.issues
		});
		return { cancelled: true };
	}
	return parsed.output;
}
