import type { FeedbackComment } from "@src/types";
import { useState, useEffect, useRef } from "react";
import { useStore } from "@nanostores/react";
import { cn } from "@helpers/cn";

import Avatar from "@ds/Avatar";
import ReactionBar from "@components/idea/reactions/ReactionBar";
import { formatISO8601ToHumanReadable } from "@helpers/helpers";
import { $userState } from "@components/global/userStore";

interface IdeaFeedbackCardProps {
	feedback: FeedbackComment & {
		ideaId: string;
		hid: string | number;
		creatorEmail?: string | null;
		creatorName?: string | null;
		createdAt?: string | null;
	};
	focusedIdea?: (FeedbackComment & { ideaId: string }) | null;
	focusSection: (feedback: FeedbackComment & { hid: string | number }) => void;
	/** the user's own reaction emojis on this comment (local only) */
	myReactions: string[];
	onToggleReaction: (emoji: string) => void;
}

export default function IdeaFeedbackCard({
	feedback,
	focusedIdea,
	focusSection,
	myReactions,
	onToggleReaction
}: IdeaFeedbackCardProps) {
	const { ideaId, creatorEmail, creatorName, createdAt, feedbackText } = feedback;
	const authorName = creatorName ?? creatorEmail ?? null;
	// dates render in the stored user timezone (OS zone until one is set)
	const { timezone } = useStore($userState);
	const ref = useRef<HTMLDivElement | null>(null);
	const [isTruncated, setIsTruncated] = useState(false);
	// derived: this card is the one the sidebar currently has focused,
	// identified by its stable comment id (never by array identity, which
	// breaks on every re-render)
	const isFocused = feedback.commentId != null && focusedIdea?.commentId === feedback.commentId;
	const [isShowingMore, setIsShowingMore] = useState(false);

	useEffect(() => {
		const { offsetHeight, scrollHeight } = ref.current || {};

		if (offsetHeight && scrollHeight && offsetHeight < scrollHeight) {
			setIsTruncated(true);
		} else {
			setIsTruncated(false);
		}

		// Currently focused -> scroll into view
		if (feedback.commentId != null && focusedIdea?.commentId === feedback.commentId) {
			ref.current?.scrollIntoView({ behavior: "smooth", block: "center" });
		}
	}, [ref, focusedIdea, feedback.commentId]);

	const containerClasses = cn(
		"border-stone-200",
		isFocused && "outline outline-accent-500 outline-2",
		"m-1 p-3 border rounded-lg shadow transition-all ease-in-out duration-300"
	);

	function feedbackClicked() {
		focusSection?.(feedback);
	}

	// A real button (keyboard focus + activation for free) with the
	// show-more toggle and the reactions OUTSIDE it - a button inside a
	// button is invalid.
	return (
		<div id={ideaId} className={containerClasses}>
			<button
				type="button"
				className="w-full text-left cursor-pointer"
				onClick={feedbackClicked}
			>
				<div className="flex mb-2 items-center">
					<Avatar
						style="mr-1"
						id={creatorEmail ?? undefined}
						charToShow={(authorName ?? "?").charAt(0)}
						size={"6"}
					/>
					{authorName && (
						<p className="ml-1 text-xs font-medium text-stone-700">{authorName}</p>
					)}
					<p className="ml-1 text-xs text-stone-500">
						{formatISO8601ToHumanReadable(
							createdAt ?? "",
							undefined,
							timezone ?? undefined
						)}
					</p>
				</div>
				<p
					ref={ref}
					className={cn("text-sm leading-snug", !isShowingMore && "line-clamp-5")}
				>
					{feedbackText}
				</p>
			</button>
			{isTruncated && (
				<button
					type="button"
					className="mt-2 text-xs text-stone-500"
					onClick={() => setIsShowingMore((prev) => !prev)}
				>
					{isShowingMore ? "show less" : "show more"}
				</button>
			)}
			<ReactionBar
				className="mt-2"
				label={`comment from ${authorName ?? "someone"}`}
				reactions={myReactions.map((emoji) => ({ emoji, mine: true }))}
				onToggle={onToggleReaction}
			/>
		</div>
	);
}
