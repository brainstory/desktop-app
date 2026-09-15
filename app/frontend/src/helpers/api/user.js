import { invoke } from "@tauri-apps/api/core";

/** Get user name, timezone and account creation date (all local) */
export async function getUserApi() {
	const response = await invoke("get_user");
	return {
		email: response.email,
		name: response?.name,
		mailVerified: response?.mail_verified,
		timezone: response?.timezone,
		createdAt: response?.created_at
	};
}

export async function getUserDailyStatusApi() {
	const response = await invoke("get_daily_status");
	return {
		logId: response.log_id,
		intentIdeaId: response.intent_idea_id,
		surveyId: response.survey_id,
		isCompleted: response.is_completed,
		streak: response.streak
	};
}

/**
 * Distill a result document into a short preview line for the library grid.
 * Total (never crashes on missing/empty results) and pure, so it can be
 * unit-tested.
 */
export function stripResultPreview(str) {
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
	// only append the ellipsis when something was actually cut off
	return trimmed.length > 100 ? trimmed.substring(0, 100).trim() + "..." : trimmed;
}

/** Get all ideas that the user created */
export async function getAllIdeasApi() {
	const response = await invoke("get_all_ideas");

	const displayDraftSummary = (idea) => {
		if (idea?.result === "") {
			// return the last idea transcript where the role is "user"
			// and the content is not empty
			return idea?.transcript
				?.filter((transcript) => transcript.role === "user" && transcript.content !== "")
				?.pop()?.content;
		}
	};

	return response?.ideas.map((idea) => ({
		id: idea.id,
		title: idea?.title,
		summaryPreview: stripResultPreview(idea?.result),
		createdAt: idea?.created_at,
		creatorEmail: idea?.creator_email,
		creatorName: idea?.creator_name,
		isUnread: idea?.is_unread,
		feedback: idea?.feedback,
		draftSummary: displayDraftSummary(idea)
	}));
}