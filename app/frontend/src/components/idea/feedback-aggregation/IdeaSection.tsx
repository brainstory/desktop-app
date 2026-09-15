import type { ResultSection, FeedbackComment, IdeaFeedbackItem } from "@src/types";
import IdeaDocument from "@components/idea/feedback-aggregation/IdeaDocument";
import IdeaSidebar from "@components/idea/feedback-aggregation/IdeaSidebar";
import { useState, useRef } from "react";

interface IdeaSectionProps {
	resultSections: ResultSection[];
	ideaFeedbackChildren?: IdeaFeedbackItem[] | null;
	headingIdxToComments: Record<number, FeedbackComment[]>;
	canShare: boolean;
}

export default function IdeaSection({
	resultSections,
	ideaFeedbackChildren: _ideaFeedbackChildren,
	headingIdxToComments,
	canShare
}: IdeaSectionProps) {
	// This should be used to tell the side bar which reaction should be scrolled into view
	type FocusedFeedback = FeedbackComment & { ideaId: string };
	const [focusedFeedback, setFocusedFeedback] = useState<FocusedFeedback | null>(null);
	const [focusedSection, setFocusedSection] = useState<string | number | null>(null);
	const focusFeedbackTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const focusSectionTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

	function onDocumentReactionClick(reaction: FocusedFeedback): void {
		if (focusFeedbackTimeoutRef.current) {
			clearTimeout(focusFeedbackTimeoutRef.current);
		}

		setFocusedFeedback(reaction);

		// Set a timeout to reset focusedSection to null after 3 seconds
		focusFeedbackTimeoutRef.current = setTimeout(() => {
			setFocusedFeedback(null);
		}, 3000);
	}

	function onFeedbackClick(feedback: { hid: string | number }): void {
		if (focusSectionTimeoutRef.current) {
			clearTimeout(focusSectionTimeoutRef.current);
		}

		setFocusedSection(feedback.hid);

		// Set a timeout to reset focusedSection to null after 3 seconds
		focusSectionTimeoutRef.current = setTimeout(() => {
			setFocusedSection(null);
		}, 3000);
	}

	return (
		<div className="flex h-[calc(100vh-11.5rem)]">
			<IdeaDocument
				focusedSection={focusedSection}
				onReactionClick={onDocumentReactionClick as (reaction: unknown) => void}
				currentFocusedFeedback={focusedFeedback}
				resultSections={resultSections}
				headingIdxToComments={headingIdxToComments}
			/>
			<IdeaSidebar
				currentFocusedFeedback={focusedFeedback}
				headingIdxToComments={headingIdxToComments}
				onFeedbackClick={onFeedbackClick}
				canShare={canShare}
			/>
		</div>
	);
}
