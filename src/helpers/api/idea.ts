import { Channel } from "@tauri-apps/api/core";
import { invokeCommand } from "@src/tauri/invoke";
import { CHAT_TYPE, TOPICS, type ChatType } from "@src/const";
import type { ChatMessage, IdeaDetail, IdeaFeedbackItem } from "@src/types";
import type { GenerationResult } from "@helpers/chat";

/** Raw idea as serialized by the backend (snake_case). */
export interface RawIdea {
	id: string;
	title?: string;
	type?: string;
	created_at?: string;
	creator_email?: string | null;
	creator_name?: string | null;
	is_unread?: boolean;
	transcript?: ChatMessage[];
	result?: string;
	parent_idea?: RawIdea | null;
	result_json?: unknown;
	notes?: { id: string; text: string; created_at: string }[];
}

import { getQueryParam } from "@helpers/helpers";
import { draftSummaryOf, stripResultPreview } from "@helpers/ideas";
import { QUERY_PARAMS } from "@src/tauri/commands";

/** Get idea */
export async function getIdeaApi(idea_id: string): Promise<IdeaDetail> {
	const response = await invokeCommand("getIdea", { ideaId: idea_id });

	const parentIdea = response?.parent_idea
		? {
				id: response.parent_idea.id,
				title: response.parent_idea?.title,
				createdAt: response.parent_idea?.created_at,
				creatorEmail: response.parent_idea?.creator_email,
				creatorName: response.parent_idea?.creator_name,
				isUnread: response.parent_idea?.is_unread
			}
		: null;

	return {
		id: response.id,
		title: response?.title,
		type: response?.type,
		createdAt: response?.created_at,
		creatorEmail: response?.creator_email,
		creatorName: response?.creator_name,
		isUnread: response?.is_unread,
		transcript: response?.transcript,
		summary: response?.result,
		parentIdea: parentIdea,
		resultJson: (response?.result_json ?? null) as IdeaDetail["resultJson"],
		notes:
			response?.notes?.map((note) => ({
				id: note.id,
				text: note.text,
				createdAt: note.created_at
			})) ?? null
	};
}

export interface RawFeedbackChild {
	id: string;
	title?: string;
	result?: string;
	created_at: string;
	creator_email?: string;
	creator_name?: string;
	is_unread?: boolean;
	transcript?: ChatMessage[];
	// free-form JSON from the database: model-generated, imported or
	// written by an older build - unknown shape, validate before mapping
	// (older items may still carry the LLM's emoji `labels`; they are
	// deliberately not mapped - reactions are chosen by people)
	structured_result?: unknown;
}

/**
 * Map a feedback idea's stored structured_result into display comments,
 * treating the stored JSON as unknown-typed. The contract (see the
 * feedback JSON prompt and the share import) is an object with a
 * feedback_items array of members carrying string oid_heading_text and
 * feedback_text. A non-object document or non-array container yields no
 * comments; unusable MEMBERS (null/number/object members, non-string
 * heading or empty/non-string text) are skipped one by one so valid
 * members beside them stay visible. A skipped member never renumbers
 * the survivors: itemIndex keeps the member's ORIGINAL position in the
 * stored array because comment reactions are keyed by it. An
 * absent/null oid_heading_text stays a comment (aggregation logs and
 * skips it); a non-string one is dropped instead of crashing
 * parseHeadingIndex at render time. A non-array matched_spans degrades
 * to [] - it is display-only.
 */
function feedbackCommentsOf(structuredResult: unknown): IdeaFeedbackItem["feedbackComments"] {
	if (typeof structuredResult !== "object" || structuredResult === null) return [];
	if (Array.isArray(structuredResult)) return [];
	const { feedback_items } = structuredResult as { feedback_items?: unknown };
	if (!Array.isArray(feedback_items)) return [];
	const comments: IdeaFeedbackItem["feedbackComments"] = [];
	feedback_items.forEach((member: unknown, itemIndex: number) => {
		if (typeof member !== "object" || member === null || Array.isArray(member)) return;
		const { oid_heading_text, feedback_text, matched_spans } = member as {
			oid_heading_text?: unknown;
			feedback_text?: unknown;
			matched_spans?: unknown;
		};
		if (
			oid_heading_text !== undefined &&
			oid_heading_text !== null &&
			typeof oid_heading_text !== "string"
		) {
			return;
		}
		if (typeof feedback_text !== "string" || feedback_text.trim() === "") return;
		comments.push({
			oidHeadingText: typeof oid_heading_text === "string" ? oid_heading_text : undefined,
			matchedSpans: Array.isArray(matched_spans) ? matched_spans : [],
			feedbackText: feedback_text,
			itemIndex
		});
	});
	return comments;
}

/** Get idea's children (ideas that branched off from idea_id) */
export async function getIdeaChildrenApi(idea_id: string): Promise<IdeaFeedbackItem[]> {
	const response = await invokeCommand("getIdeaChildren", { ideaId: idea_id });

	return response?.ideas.map((idea) => {
		return {
			id: idea.id,
			title: idea?.title,
			summaryPreview: stripResultPreview(idea?.result),
			// an unfinished feedback session (no result yet)
			isDraft: !idea?.result,
			createdAt: idea?.created_at,
			creatorEmail: idea?.creator_email,
			creatorName: idea?.creator_name,
			isUnread: idea?.is_unread,
			feedbackComments: feedbackCommentsOf(idea?.structured_result),
			draftSummary: draftSummaryOf(idea?.result, idea?.transcript)
		};
	});
}

/**
 * Create an idea saved in the local database.
 * Called after user is finished with an idea and a summary is generated.
 * @returns created idea's uuid
 */
export async function createIdeaApi(
	result: string,
	transcript: ChatMessage[],
	ideaType: ChatType = CHAT_TYPE.ORIGINAL,
	parentIdeaId: string | null = null,
	logId: string | null = null,
	ideaMetadata: Record<string, unknown> = {}
): Promise<string> {
	const i = getQueryParam(QUERY_PARAMS.topic);
	if (i !== null) {
		ideaMetadata.suggestion = {
			index: i,
			topic: TOPICS[Number(i)]
		};
	}

	const response = await invokeCommand("createIdea", {
		result,
		transcript,
		parentIdeaId,
		ideaMetadata,
		ideaType,
		logId
	});
	return response.id;
}

/** Update an idea saved in the database.
 * Called whenever a draft is saved or the final result is stored.
 * `result` is only sent when the caller provides it: the backend
 * leaves keys that are absent alone, while a transcript-only autosave
 * defaulting it to "" would wipe a stored summary and its derived
 * title.
 * @returns updated idea's uuid
 */
export async function updateIdeaApi(
	ideaId: string,
	transcript: ChatMessage[],
	options?: {
		result?: string;
		structuredResult?: unknown;
	}
): Promise<string> {
	const response = await invokeCommand("updateIdea", {
		id: ideaId,
		transcript,
		structuredResult: options?.structuredResult ?? null,
		...(options?.result !== undefined && { result: options.result })
	});
	return response.id;
}

/** Link (or relink) an idea to a daily log that was submitted after the
 * idea was created - the daily-intent session's backfill path. */
export function linkIdeaLogApi(ideaId: string, logId: string): Promise<null> {
	return invokeCommand("linkIdeaLog", { ideaId, logId });
}

/** Regenerate an idea's summary from its stored transcript; events
 * (status/chunk/cumulative) arrive on the returned channel, the invoke
 * resolves with the generation result once the new summary is saved. */
export function regenerateResultApi(ideaId: string): {
	channel: Channel;
	invokePromise: Promise<GenerationResult>;
} {
	const channel = new Channel();
	const invokePromise = invokeCommand("regenerateResult", { ideaId, onEvent: channel });
	return { channel, invokePromise };
}

/** Replace an idea's summary with a manually edited one. */
export function setIdeaResultApi(ideaId: string, result: string): Promise<null> {
	return invokeCommand("setIdeaResult", { ideaId, result });
}

/** Update an idea title saved in the database.
 * Called after user edits idea title
 * @returns created idea's uuid
 */
export async function updateIdeaTitleApi(ideaId: string, title: string): Promise<string> {
	const response = await invokeCommand("updateIdea", {
		id: ideaId,
		title
	});
	return response.id;
}

/** Mark idea read by user. Only applies to ideas imported from others. */
export async function markIdeaReadApi(idea_id: string): Promise<unknown> {
	const response = await invokeCommand("markIdeaRead", { ideaId: idea_id });
	return response;
}

/** Permanently delete an idea (and its feedback children). */
export function deleteIdeaApi(idea_id: string): Promise<unknown> {
	return invokeCommand("deleteIdea", { ideaId: idea_id });
}

/** Append a note to an idea's writeup ("## Notes" section). */
export function addIdeaNoteApi(ideaId: string, note: string): Promise<unknown> {
	return invokeCommand("addIdeaNote", { ideaId, note });
}

/** Replace one note's text. */
export function updateIdeaNoteApi(ideaId: string, noteId: string, note: string): Promise<unknown> {
	return invokeCommand("updateIdeaNote", { ideaId, noteId, note });
}

/** Delete one note. */
export function deleteIdeaNoteApi(ideaId: string, noteId: string): Promise<unknown> {
	return invokeCommand("deleteIdeaNote", { ideaId, noteId });
}

export default {
	getIdeaApi,
	getIdeaChildrenApi,
	createIdeaApi,
	markIdeaReadApi,
	updateIdeaTitleApi,
	setIdeaResultApi,
	linkIdeaLogApi,
	deleteIdeaApi,
	addIdeaNoteApi,
	updateIdeaNoteApi,
	deleteIdeaNoteApi
};
