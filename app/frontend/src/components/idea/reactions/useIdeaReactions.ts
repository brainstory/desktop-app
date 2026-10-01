import { useEffect, useRef, useState } from "react";
import {
	getReactionsApi,
	toggleSectionReactionApi,
	type IdeaReactions,
	type SectionReaction
} from "@helpers/api/reactions";

const NO_REACTIONS: IdeaReactions = { sections: [], comments: [] };

export const REACTION_ERROR = "Error: Could not save your reaction";

/** `reactions` with the user's own section reaction set to `isOn`. */
function withSectionReaction(
	reactions: IdeaReactions,
	sectionIndex: number,
	emoji: string,
	isOn: boolean
): IdeaReactions {
	const sections = reactions.sections.filter(
		(r) => !(r.mine && r.sectionIndex === sectionIndex && r.emoji === emoji)
	);
	if (isOn) {
		sections.push({ sectionIndex, emoji, mine: true, from: null });
	}
	return { ...reactions, sections };
}

/**
 * The reactions on one idea page: loaded once with the idea, toggled
 * optimistically. Each toggle applies the backend's returned state; when
 * toggles on the same target race, only the latest one's answer (or
 * rollback) is applied. A failed toggle rolls back and reports
 * `REACTION_ERROR` through `onError`.
 */
export function useIdeaReactions(
	ideaId: string | undefined,
	onError: (message: string) => void
): {
	sectionReactions: SectionReaction[];
	toggleSectionReaction: (sectionIndex: number, emoji: string) => void;
} {
	const [reactions, setReactions] = useState<IdeaReactions>(NO_REACTIONS);
	// latest request number per target, to drop stale answers
	const latestRequest = useRef(new Map<string, number>());

	useEffect(() => {
		if (!ideaId) return;
		let isCurrent = true;
		getReactionsApi(ideaId)
			.then((res) => {
				if (isCurrent) setReactions(res);
			})
			.catch((err) => {
				// reactions are auxiliary: the page works without them
				console.error("failed to load reactions for idea", ideaId, err);
			});
		return () => {
			isCurrent = false;
		};
	}, [ideaId]);

	/** Claims a new request for `key`; the returned check says whether it is still the latest. */
	function startRequest(key: string): () => boolean {
		const request = (latestRequest.current.get(key) ?? 0) + 1;
		latestRequest.current.set(key, request);
		return () => latestRequest.current.get(key) === request;
	}

	function toggleSectionReaction(sectionIndex: number, emoji: string): void {
		if (!ideaId) return;
		const wasOn = reactions.sections.some(
			(r) => r.mine && r.sectionIndex === sectionIndex && r.emoji === emoji
		);
		const isLatest = startRequest(`section:${sectionIndex}:${emoji}`);
		const apply = (isOn: boolean) =>
			setReactions((prev) => withSectionReaction(prev, sectionIndex, emoji, isOn));

		apply(!wasOn);
		toggleSectionReactionApi(ideaId, sectionIndex, emoji)
			.then((isOn) => {
				if (isLatest()) apply(isOn);
			})
			.catch((err) => {
				console.error("failed to toggle section reaction", err);
				if (isLatest()) apply(wasOn);
				onError(REACTION_ERROR);
			});
	}

	return { sectionReactions: reactions.sections, toggleSectionReaction };
}
