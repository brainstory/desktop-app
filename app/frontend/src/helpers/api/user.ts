import { invoke } from "@tauri-apps/api/core";
import type { DailyStatus, IdeaListItem } from "@src/types";

export interface CurrentUser {
	email: null;
	name?: string;
	mailVerified: boolean;
	timezone?: string;
	createdAt?: string;
}

/** Get user name, timezone and account creation date (all local) */
export async function getUserApi(): Promise<CurrentUser> {
	const response = await invoke<{
		email: null;
		name?: string;
		mail_verified: boolean;
		timezone?: string;
		created_at: string;
	}>("get_user");
	return {
		email: response.email,
		name: response?.name,
		mailVerified: response?.mail_verified,
		timezone: response?.timezone,
		createdAt: response?.created_at
	};
}

export async function getUserDailyStatusApi(): Promise<DailyStatus> {
	const response = await invoke<{
		log_id: string | null;
		intent_idea_id: string | null;
		survey_id: string | null;
		is_completed: boolean;
		streak: number;
	}>("get_daily_status");
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
export function stripResultPreview(str: unknown): string {
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

/** Raw idea row as serialized by the backend (snake_case). */
interface RawIdeaItem {
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
	feedback?: IdeaListItem[];
}

/** Get all ideas that the user created */
export async function getAllIdeasApi(): Promise<IdeaListItem[]> {
	const response = await invoke<{ ideas: RawIdeaItem[] }>("get_all_ideas");

	const displayDraftSummary = (idea: RawIdeaItem): string | undefined => {
		if (idea?.result === "") {
			// return the last idea transcript where the role is "user"
			// and the content is not empty
			return idea?.transcript
				?.filter((transcript) => transcript.role === "user" && transcript.content !== "")
				?.pop()?.content;
		}
		return undefined;
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
