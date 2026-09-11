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

/** Kept for compatibility; the desktop app has no trials */
export async function getUserTrial() {
	const response = await invoke("get_user_trial");
	return {
		trialEndAt: response.trial_end,
		isPaid: response.is_paid
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

/** Get all ideas that the user created */
export async function getAllIdeasApi() {
	const response = await invoke("get_all_ideas");

	const strip = (str) => {
		// remove the first line before the first \n\n,
		// and if the next line starts with ##, remove the ##
		// then replace all newlines with spaces
		const removedFirstLine = str.substring(str.indexOf("\n\n") + 2);
		const removedFirstLineAndHash = removedFirstLine.replace(/^##/, "");
		const removedNewLines = removedFirstLineAndHash.replace(/\n/g, " ");

		return removedNewLines.substring(0, 100).trim() + "...";
	};

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
		summaryPreview: strip(idea?.result),
		createdAt: idea?.created_at,
		creatorEmail: idea?.creator_email,
		creatorName: idea?.creator_name,
		isUnread: idea?.is_unread,
		sharedWithUsers: idea?.shared_with_users,
		feedback: idea?.feedback,
		draftSummary: displayDraftSummary(idea)
	}));
}

/** Get all notifications for the user (none in the desktop app) */
export async function getAllUserNotifications() {
	const response = await invoke("get_notifications");
	return response.notifications;
}
