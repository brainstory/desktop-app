import type { ResultSection, FeedbackComment } from "@src/types";
import type { SectionReaction } from "@helpers/api/reactions";
import IdeaDocument from "@components/idea/feedback-aggregation/IdeaDocument";
import IdeaSidebar from "@components/idea/feedback-aggregation/IdeaSidebar";
import { useState } from "react";
import { useTimeout } from "@src/hooks/useTimeout";

interface IdeaSectionProps {
	resultSections: ResultSection[];
	headingIdxToComments: Record<number, FeedbackComment[]>;
	canShare: boolean;
	sectionReactions: SectionReaction[];
	onToggleSectionReaction: (sectionIndex: number, emoji: string) => void;
}

export default function IdeaSection({
	resultSections,
	headingIdxToComments,
	canShare,
	sectionReactions,
	onToggleSectionReaction
}: IdeaSectionProps) {
	// This should be used to tell the side bar which comment should be scrolled into view
	type FocusedFeedback = FeedbackComment & { ideaId: string };
	const [focusedFeedback, setFocusedFeedback] = useState<FocusedFeedback | null>(null);
	const [focusedSection, setFocusedSection] = useState<string | number | null>(null);
	const [scheduleFeedbackReset] = useTimeout();
	const [scheduleSectionReset] = useTimeout();

	function onDocumentCommentClick(comment: FocusedFeedback): void {
		setFocusedFeedback(comment);
		// reset the highlight after 3 seconds (the timer dies with the section)
		scheduleFeedbackReset(() => setFocusedFeedback(null), 3000);
	}

	function onFeedbackClick(feedback: { hid: string | number }): void {
		setFocusedSection(feedback.hid);
		// reset the section highlight after 3 seconds
		scheduleSectionReset(() => setFocusedSection(null), 3000);
	}

	return (
		<div className="flex h-[calc(100vh-11.5rem)]">
			<IdeaDocument
				focusedSection={focusedSection}
				onCommentClick={onDocumentCommentClick as (comment: unknown) => void}
				resultSections={resultSections}
				headingIdxToComments={headingIdxToComments}
				sectionReactions={sectionReactions}
				onToggleSectionReaction={onToggleSectionReaction}
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
