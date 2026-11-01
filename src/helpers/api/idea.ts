import { Channel } from "@tauri-apps/api/core";
import { invokeCommand } from "@src/tauri/invoke";
import { CHAT_TYPE, TOPICS, type ChatType } from "@src/const";
import type { ChatMessage, IdeaDetail, IdeaFeedbackItem } from "@src/types";
import type { GenerationResult } from "@helpers/chat";
import type { IdeaItem } from "@src/tauri/generated/IdeaItem";

import { safeParse } from "valibot";
import { getQueryParam } from "@helpers/helpers";
import { FeedbackItemSchema, ResultSectionsSchema, StructuredFeedbackSchema } from "./schemas";
import { draftSummaryOf, stripResultPreview } from "@helpers/ideas";
import { QUERY_PARAMS } from "@src/tauri/commands";

/** Validate a stored result document; null for absent or malformed. */
function parseResultJson(blob: IdeaItem["result_json"], ideaId: string): IdeaDetail["resultJson"] {
	if (blob === null || blob === undefined) return null;
	const parsed = safeParse(ResultSectionsSchema, blob);
	if (!parsed.success) {
		console.warn(`idea ${ideaId}: stored result document is malformed; ignoring it`);
		return null;
	}
	return parsed.output;
}

/** Get idea */
export async function getIdeaApi(idea_id: string): Promise<IdeaDetail> {
	const response: IdeaItem = await invokeCommand("getIdea", { ideaId: idea_id });

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
		// stored free JSON: validate on read, a malformed document reads
		// as missing rather than crashing the idea page
		resultJson: parseResultJson(response?.result_json, idea_id),
		notes:
			response?.notes?.map((note) => ({
				id: note.id,
				text: note.text,
				createdAt: note.created_at
			})) ?? null
	};
}

/**
 * Map a feedback idea's stored structured_result into display comments.
 * The stored JSON is unknown-typed, so the document shape and each
 * member are validated with the schemas (see ./schemas); the schema
 * rules ARE the contract: an object with a feedback_items array of
 * members carrying string oid_heading_text and feedback_text.
 */
function feedbackCommentsOf(
	structuredResult: IdeaItem["structured_result"],
	ideaId: string
): IdeaFeedbackItem["feedbackComments"] {
	if (structuredResult === null || structuredResult === undefined) return [];
	const document = safeParse(StructuredFeedbackSchema, structuredResult);
	if (!document.success) {
		console.warn(`idea ${ideaId}: structured feedback document is malformed; ignoring it`);
		return [];
	}
	const comments: IdeaFeedbackItem["feedbackComments"] = [];
	document.output.feedback_items.forEach((member: unknown, itemIndex: number) => {
		// unusable members are skipped one by one so valid members beside
		// them stay visible; a skipped member never renumbers the
		// survivors: itemIndex keeps the member's ORIGINAL position in
		// the stored array because comment reactions are keyed by it
		const parsed = safeParse(FeedbackItemSchema, member);
		if (!parsed.success) return;
		comments.push({ ...parsed.output, itemIndex });
	});
	return comments;
}

/** Get idea's children (ideas that branched off from idea_id) */
export async function getIdeaChildrenApi(idea_id: string): Promise<IdeaFeedbackItem[]> {
	const response: { ideas: IdeaItem[] } = await invokeCommand("getIdeaChildren", {
		ideaId: idea_id
	});

	return response?.ideas.map((idea) => {
		return {
			id: idea.id,
			type: idea?.type,
			title: idea?.title,
			summaryPreview: stripResultPreview(idea?.result),
			// an unfinished feedback session (no result yet)
			isDraft: !idea?.result,
			createdAt: idea?.created_at,
			creatorEmail: idea?.creator_email,
			creatorName: idea?.creator_name,
			isUnread: idea?.is_unread,
			feedbackComments: feedbackCommentsOf(idea?.structured_result, idea.id),
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
export function setIdeaResultApi(
	ideaId: string,
	result: string,
	expectedPreviousResult?: string
): Promise<null> {
	return invokeCommand("setIdeaResult", { ideaId, result, expectedPreviousResult });
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
