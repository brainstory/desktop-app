import type { FeedbackComment } from "@src/types";
import { useState, useEffect, useRef } from "react";

import EmojiItem from "@components/idea/feedback-aggregation/EmojiItem";
import { formatISO8601ToHumanReadable } from "@helpers/helpers";

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
}

export default function IdeaFeedbackCard({ feedback, focusedIdea, focusSection }: IdeaFeedbackCardProps) {
	const { ideaId, creatorEmail, creatorName, createdAt, feedbackText, labels } = feedback;
	const ref = useRef<HTMLDivElement | null>(null);
	const [isTruncated, setIsTruncated] = useState(false);
	// derived: this card is the one the sidebar currently has focused,
	// identified by its stable comment id (never by array identity, which
	// breaks on every re-render)
	const isFocused =
		feedback.commentId != null && focusedIdea?.commentId === feedback.commentId;
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

	const containerClasses = `border-stone-200 ${
		isFocused && "outline outline-blue-500 outline-2"
	} m-1 p-3 border rounded-lg shadow transition-all ease-in-out duration-300`;

	function feedbackClicked() {
		focusSection?.(feedback);
	}

	return (
		<div id={ideaId} className={containerClasses} role="button" onClick={feedbackClicked}>
			<div className="flex mb-2 items-end">
				<EmojiItem
					ideaId={ideaId}
					labels={labels}
					creatorEmail={creatorEmail}
					creatorName={creatorName}
					isBlue={false}
					labelsHasBorder={true}
					style="mr-1"
				/>
				<p className="ml-1 text-xs text-stone-500">
				{formatISO8601ToHumanReadable(createdAt ?? "")}
			</p>
			</div>
			<p ref={ref} className={`text-sm leading-snug ${!isShowingMore && "line-clamp-5"}`}>
				{feedbackText}
			</p>
			{isTruncated && (
				<button
					className="mt-2 text-xs text-stone-500"
					onClick={(event) => {
						event.stopPropagation();
						setIsShowingMore((prev) => !prev);
					}}
				>
					{isShowingMore ? "show less" : "show more"}
				</button>
			)}
		</div>
	);
}
