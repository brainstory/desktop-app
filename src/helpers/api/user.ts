import { invokeCommand } from "@src/tauri/invoke";
import { draftSummaryOf, stripResultPreview } from "@helpers/ideas";
import type { DailyStatus, IdeaListItem } from "@src/types";
import type { DailyStatus as RawDailyStatus } from "@src/tauri/generated/DailyStatus";
import type { IdeaItem } from "@src/tauri/generated/IdeaItem";
import type { UserData } from "@src/tauri/generated/UserData";

export interface CurrentUser {
	name?: string;
	timezone?: string;
	createdAt?: string;
}

/** Get user name, timezone and account creation date (all local).
 * Leftover account fields the backend may still send (email,
 * mail_verified) are deliberately not read. */
export async function getUserApi(): Promise<CurrentUser> {
	const response: UserData = await invokeCommand("getUser");
	return {
		name: response?.name ?? undefined,
		timezone: response?.timezone ?? undefined,
		createdAt: response?.created_at
	};
}

export async function getUserDailyStatusApi(): Promise<DailyStatus> {
	const response: RawDailyStatus = await invokeCommand("getDailyStatus");
	return {
		logId: response.log_id ?? null,
		intentIdeaId: response.intent_idea_id ?? null,
		surveyId: response.survey_id ?? null,
		isCompleted: response.is_completed,
		streak: response.streak
	};
}

function toIdeaListItem(idea: IdeaItem): IdeaListItem {
	return {
		id: idea.id,
		type: idea.type,
		title: idea?.title,
		summaryPreview: stripResultPreview(idea?.result),
		// explicit draft flag: the old "summaryPreview === '...'" sentinel
		// could never match (stripResultPreview never returns '...')
		isDraft: !idea?.result,
		createdAt: idea?.created_at,
		creatorEmail: idea?.creator_email,
		creatorName: idea?.creator_name,
		isUnread: idea?.is_unread,
		// mapped like their parents, so a feedback draft is flagged as one
		feedback: idea?.feedback?.map(toIdeaListItem),
		draftSummary: draftSummaryOf(idea?.result, idea?.transcript)
	};
}

/** Get all ideas that the user created */
export async function getAllIdeasApi(): Promise<IdeaListItem[]> {
	const response = await invokeCommand("getAllIdeas");
	return response?.ideas.map(toIdeaListItem);
}
