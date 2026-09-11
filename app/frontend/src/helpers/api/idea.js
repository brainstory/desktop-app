import { invoke } from "@tauri-apps/api/core";
import { TOPICS } from "@src/const";

import { getQueryParam } from "@helpers/helpers";

/** Get idea */
export async function getIdeaApi(idea_id) {
	const response = await invoke("get_idea", { ideaId: idea_id });

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
		resultJson: response?.result_json
	};
}

/** Get idea's children (ideas that branched off from idea_id) */
export async function getIdeaChildrenApi(idea_id) {
	const response = await invoke("get_idea_children", { ideaId: idea_id });

	const strip = (str) => {
		// remove the first line before the first \n\n,
		// and if the next line starts with ##, remove the ##
		// then replace all newlines with spaces
		const removedFirstLine = str.substring(str.indexOf("\n\n") + 2);
		const removedFirstLineAndHash = removedFirstLine.replace(/^##/, "");
		const removedNewLines = removedFirstLineAndHash.replace(/\n/g, " ");

		return removedNewLines.substring(0, 100).trim() + "...";
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
	result,
	transcript,
	ideaType = "original",
	parentIdeaId = null,
	logId = null,
	ideaMetadata = {}
) {
	const i = getQueryParam("topic");
	if (i !== null) {
		ideaMetadata.suggestion = {
			index: i,
			topic: TOPICS[i]
		};
	}

	const response = await invoke("create_idea", {
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
export async function updateIdeaApi(ideaId, transcript, result = "", structuredResult = null) {
	const response = await invoke("update_idea", {
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
export async function updateIdeaTitleApi(ideaId, title) {
	const response = await invoke("update_idea", {
		id: ideaId,
		title
	});
	return response.id;
}

/** Mark idea read by user. Only applies to ideas imported from others. */
export async function markIdeaReadApi(idea_id) {
	const response = await invoke("mark_idea_read", { ideaId: idea_id });
	return response;
}

/** Permanently delete an idea (and its feedback children). */
export async function deleteIdeaApi(idea_id) {
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
