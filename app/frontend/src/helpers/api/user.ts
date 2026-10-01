import { invokeCommand } from "@src/tauri/invoke";
import { draftSummaryOf, stripResultPreview } from "@helpers/ideas";
import type { DailyStatus, IdeaListItem } from "@src/types";

export interface CurrentUser {
	name?: string;
	timezone?: string;
	createdAt?: string;
}

/** The local user as serialized by the backend (snake_case). Leftover
 * account fields the backend may still send (email, mail_verified) are
 * deliberately not read. */
export interface RawUser {
	name?: string;
	timezone?: string;
	created_at: string;
}

/** Get user name, timezone and account creation date (all local) */
export async function getUserApi(): Promise<CurrentUser> {
	const response = await invokeCommand("getUser");
	return {
		name: response?.name,
		timezone: response?.timezone,
		createdAt: response?.created_at
	};
}

/** Today's log/intent status as serialized by the backend. */
export interface RawDailyStatus {
	log_id: string | null;
	intent_idea_id: string | null;
	survey_id: string | null;
	is_completed: boolean;
	streak: number;
}

export async function getUserDailyStatusApi(): Promise<DailyStatus> {
	const response = await invokeCommand("getDailyStatus");
	return {
		logId: response.log_id,
		intentIdeaId: response.intent_idea_id,
		surveyId: response.survey_id,
		isCompleted: response.is_completed,
		streak: response.streak
	};
}

/** Raw idea row as serialized by the backend (snake_case). */
export interface RawIdeaItem {
	id: string;
	title: string;
	result?: string;
	type?: string;
	created_at: string;
	creator_email?: string;
	creator_name?: string;
	is_unread?: boolean;
	transcript?: { role: string; content: string }[];
	shared_with_users?: string[];
	/** feedback children, same row shape (the list sends a transcript
	 * only for drafts); only top-level ideas carry this */
	feedback?: RawIdeaItem[];
}

function toIdeaListItem(idea: RawIdeaItem): IdeaListItem {
	return {
		id: idea.id,
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
