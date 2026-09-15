/** Shared domain shapes mirrored from the Rust backend's types. */

export interface ChatMessage {
	role: string;
	content: string;
}

/** One markdown section of a generated result document. */
export interface ResultSection {
	heading: string;
	body: string;
}

/** A structured feedback document attached to imported feedback ideas. */
export interface StructuredFeedback {
	[key: string]: unknown;
}

/** A feedback comment grouped under a result-document heading. */
export interface FeedbackComment {
	ideaId?: string;
	creatorEmail?: string | null;
	creatorName?: string | null;
	createdAt?: string | null;
	oidHeadingText?: string;
	matchedSpans: unknown[];
	feedbackText: string;
	labels: FeedbackLabel[];
}

export interface FeedbackLabel {
	name: string;
	emoji: string;
}

/** Attribution info shared by ideas and feedback cards. */
export interface CreatorInfo {
	creatorEmail?: string | null;
	creatorName?: string | null;
	createdAt?: string | null;
}

/** Item in the library grid (getAllIdeasApi). */
export interface IdeaListItem extends CreatorInfo {
	id: string;
	title?: string | null;
	summaryPreview: string;
	isUnread?: boolean | null;
	feedback?: IdeaListItem[] | null;
	draftSummary?: string | null;
}

/** Full idea returned by getIdeaApi. */
export interface IdeaDetail extends CreatorInfo {
	id: string;
	title?: string | null;
	type?: string | null;
	isUnread?: boolean | null;
	transcript?: ChatMessage[] | null;
	summary?: string | null;
	sharedWithUsers?: unknown[] | null;
	parentIdea?: (CreatorInfo & { id: string; title?: string | null }) | null;
	resultJson?: ResultSection[] | null;
}

/** Feedback child item returned by getIdeaChildrenApi. */
export interface IdeaFeedbackItem extends CreatorInfo {
	id: string;
	title?: string | null;
	summaryPreview: string;
	isUnread?: boolean | null;
	feedbackComments: FeedbackComment[];
}

export interface DailyStatus {
	logId: string | null;
	intentIdeaId: string | null;
	surveyId: string | null;
	isCompleted: boolean;
	streak: number;
}
