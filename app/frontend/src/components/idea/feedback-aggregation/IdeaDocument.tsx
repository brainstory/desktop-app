import type { ResultSection, FeedbackComment } from "@src/types";
import { useRef, useEffect } from "react";
import ReactMarkdown from "react-markdown";
import EmojiList from "@components/idea/feedback-aggregation/EmojiList";

interface IdeaDocumentProps {
	resultSections: ResultSection[];
	headingIdxToComments: Record<number, FeedbackComment[]>;
	onReactionClick: (reaction: unknown) => void;
	focusedSection?: string | number | null;
	currentFocusedFeedback?: (FeedbackComment & { ideaId: string }) | null;
}

export default function IdeaDocument({
	resultSections,
	headingIdxToComments,
	onReactionClick,
	focusedSection
}: IdeaDocumentProps) {
	const containerRef = useRef<HTMLDivElement | null>(null);

	useEffect(() => {
		// Scroll into view when focusedSection matches outerIndex
		if (containerRef.current && focusedSection !== null) {
			const headingRef = containerRef.current.querySelector(`#emojiList-${focusedSection ?? ""}`);
			if (headingRef) {
				headingRef.scrollIntoView({ behavior: "smooth", block: "end" });
			}
		}
	}, [focusedSection]);

	return (
		<div
			className={
				"w-[calc(100%-20.5rem)] px-7 pb-7 overflow-y-auto prose max-w-none text-sm md:text-lg prose-stone prose-headings:underline prose-headings:decoration-pink-500 prose-headings:text-xl prose-h1:no-underline prose-h1:mb-1 prose-h1:text-xl prose-h1:lg:text-2xl prose-p:mb-1 " +
				"prose-p:leading-normal lg:prose-p:leading-loose"
			}
			ref={containerRef}
		>
			{(resultSections ?? []).map((section: ResultSection, outerIndex: number) => (
				<div key={outerIndex} id={`heading-${outerIndex}`}>
					<ReactMarkdown>{section.heading}</ReactMarkdown>
					<ReactMarkdown>{section.body}</ReactMarkdown>
					<div id={`emojiList-${outerIndex}`}>
						<EmojiList
							reactions={headingIdxToComments[outerIndex]}
							onReactionClick={onReactionClick}
							isFocused={Number(focusedSection) === outerIndex}
						/>
					</div>
				</div>
			))}
		</div>
	);
}
