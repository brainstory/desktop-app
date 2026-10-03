import { invokeCommand } from "@src/tauri/invoke";

/** One reaction on a section of an idea's result document. */
export interface SectionReaction {
	/** the section's index in the idea's result_json */
	sectionIndex: number;
	emoji: string;
	/** true for the local user's own reaction */
	mine: boolean;
	/** sender name for reactions that arrived in imported feedback */
	from: string | null;
}

/** The local user's reaction on one item of a feedback idea. */
export interface CommentReaction {
	feedbackIdeaId: string;
	/** index in the feedback idea's structured_result.feedback_items */
	itemIndex: number;
	emoji: string;
}

export interface IdeaReactions {
	sections: SectionReaction[];
	comments: CommentReaction[];
}

/**
 * Every section reaction on an idea (the user's and imported ones) plus
 * the user's reactions on the comments of its feedback children.
 */
export async function getReactionsApi(ideaId: string): Promise<IdeaReactions> {
	const response = await invokeCommand("getReactions", { ideaId });
	return {
		sections: response?.sections ?? [],
		comments: response?.comments ?? []
	};
}

/** Toggle the user's reaction on a section. Resolves true when it is now on. */
export function toggleSectionReactionApi(
	ideaId: string,
	sectionIndex: number,
	emoji: string
): Promise<boolean> {
	return invokeCommand("toggleSectionReaction", { ideaId, sectionIndex, emoji });
}

/** Toggle the user's reaction on a feedback comment. Resolves true when it is now on. */
export function toggleCommentReactionApi(
	feedbackIdeaId: string,
	itemIndex: number,
	emoji: string
): Promise<boolean> {
	return invokeCommand("toggleCommentReaction", { feedbackIdeaId, itemIndex, emoji });
}
