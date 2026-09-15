import { invoke } from "@tauri-apps/api/core";
import { TOPICS } from "@src/const";
import type { ChatMessage, IdeaDetail, IdeaFeedbackItem } from "@src/types";

/** Raw idea as serialized by the backend (snake_case). */
interface RawIdea {
	id: string;
	title?: string;
	type?: string;
	created_at?: string;
	creator_email?: string | null;
	creator_name?: string | null;
	is_unread?: boolean;
	transcript?: ChatMessage[];
	result?: string;
	shared_with_users?: unknown[];
	parent_idea?: RawIdea | null;
	result_json?: unknown;
}

import { getQueryParam } from "@helpers/helpers";

/** Get idea */
export async function getIdeaApi(idea_id: string): Promise<IdeaDetail> {
	const response = await invoke<RawIdea>("get_idea", { ideaId: idea_id });

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
		sharedWithUsers: response?.shared_with_users,
		parentIdea: parentIdea,
		resultJson: (response?.result_json ?? null) as IdeaDetail["resultJson"]
	};
}

interface RawFeedbackChild {
	id: string;
	title?: string;
	result?: string;
	created_at: string;
	creator_email?: string;
	creator_name?: string;
	is_unread?: boolean;
	structured_result?: {
		feedback_items?: {
			oid_heading_text: string;
			matched_spans: unknown[];
			feedback_text: string;
			labels: { name: string; emoji: string }[];
		}[];
	} | null;
}

/** Get idea's children (ideas that branched off from idea_id) */
export async function getIdeaChildrenApi(idea_id: string): Promise<IdeaFeedbackItem[]> {
	const response = await invoke<{ ideas: RawFeedbackChild[] }>("get_idea_children", {
		ideaId: idea_id
	});

	const strip = (str: unknown): string => {
		// remove the first line before the first \n\n,
		// and if the next line starts with ##, remove the ##
		// then replace all newlines with spaces
		if (typeof str !== "string" || str === "") return "";
		const removedFirstLine = str.includes("\n\n")
			? str.substring(str.indexOf("\n\n") + 2)
			: str;
		const removedFirstLineAndHash = removedFirstLine.replace(/^##/, "");
		const removedNewLines = removedFirstLineAndHash.replace(/\n/g, " ");
		const trimmed = removedNewLines.trim();
		if (!trimmed) return "";
		return trimmed.length > 100 ? trimmed.substring(0, 100).trim() + "..." : trimmed;
	};

	return response?.ideas.map((idea) => {
		const feedbackComments = idea?.structured_result?.feedback_items
			? idea.structured_result.feedback_items.map((feedbackComment) => ({
					oidHeadingText: feedbackComment.oid_heading_text,
					matchedSpans: feedbackComment.matched_spans,
					feedbackText: feedbackComment.feedback_text,
					labels: feedbackComment.labels
			  }))
			: [];
		return {
			id: idea.id,
			title: idea?.title,
			summaryPreview: strip(idea?.result),
			createdAt: idea?.created_at,
			creatorEmail: idea?.creator_email,
			creatorName: idea?.creator_name,
			isUnread: idea?.is_unread,
			feedbackComments: feedbackComments
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
	ideaType = "original",
	parentIdeaId: string | null = null,
	logId: string | null = null,
	ideaMetadata: Record<string, unknown> = {}
): Promise<string> {
	const i = getQueryParam("topic");
	if (i !== null) {
		ideaMetadata.suggestion = {
			index: i,
			topic: TOPICS[Number(i)]
		};
	}

	const response = await invoke<{ id: string }>("create_idea", {
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
 * @returns created idea's uuid
 */
export async function updateIdeaApi(
	ideaId: string,
	transcript: ChatMessage[],
	result = "",
	structuredResult: unknown = null
): Promise<string> {
	const response = await invoke<{ id: string }>("update_idea", {
		id: ideaId,
		transcript,
		result,
		structuredResult
	});
	return response.id;
}

/** Update an idea title saved in the database.
 * Called after user edits idea title
 * @returns created idea's uuid
 */
export async function updateIdeaTitleApi(ideaId: string, title: string): Promise<string> {
	const response = await invoke<{ id: string }>("update_idea", {
		id: ideaId,
		title
	});
	return response.id;
}

/** Mark idea read by user. Only applies to ideas imported from others. */
export async function markIdeaReadApi(idea_id: string): Promise<unknown> {
	const response = await invoke("mark_idea_read", { ideaId: idea_id });
	return response;
}

/** Permanently delete an idea (and its feedback children). */
export function deleteIdeaApi(idea_id: string): Promise<unknown> {
	return invoke("delete_idea", { ideaId: idea_id });
}

export default {
	getIdeaApi,
	getIdeaChildrenApi,
	createIdeaApi,
	markIdeaReadApi,
	updateIdeaTitleApi,
	deleteIdeaApi
};
