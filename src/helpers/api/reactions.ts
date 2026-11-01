import { invokeCommand } from "@src/tauri/invoke";
import type { CommentReaction as RawCommentReaction } from "@src/tauri/generated/CommentReaction";
import type { IdeaReactions as RawIdeaReactions } from "@src/tauri/generated/IdeaReactions";
import type { SectionReaction as RawSectionReaction } from "@src/tauri/generated/SectionReaction";

export type SectionReaction = RawSectionReaction;

export type CommentReaction = RawCommentReaction;

export type IdeaReactions = RawIdeaReactions;

/**
 * Every section reaction on an idea (the user's and imported ones) plus
 * the user's reactions on the comments of its feedback children.
 */
export async function getReactionsApi(ideaId: string): Promise<IdeaReactions> {
	const response: RawIdeaReactions = await invokeCommand("getReactions", { ideaId });
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
