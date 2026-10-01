import type { ResultSection, FeedbackComment } from "@src/types";
import { useRef, useEffect } from "react";
import ReactMarkdown from "react-markdown";
import SectionCommenters from "@components/idea/feedback-aggregation/SectionCommenters";
import ReactionBar from "@components/idea/reactions/ReactionBar";
import type { SectionReaction } from "@helpers/api/reactions";

const SECTION_REACTIONS_NOTE = "Your reactions are included when you export feedback on this idea.";

/** Plain heading text for a section's accessible label. */
function sectionLabel(section: ResultSection, index: number): string {
	const heading = section.heading.replace(/^#+/, "").trim();
	return heading || `section ${index}`;
}

/** The empty title slot (and any blank section) has nothing to react to. */
function hasContent(section: ResultSection): boolean {
	return section.heading.trim() !== "" || section.body.trim() !== "";
}

interface IdeaDocumentProps {
	resultSections: ResultSection[];
	headingIdxToComments: Record<number, FeedbackComment[]>;
	onCommentClick: (comment: unknown) => void;
	focusedSection?: string | number | null;
	sectionReactions: SectionReaction[];
	onToggleSectionReaction: (sectionIndex: number, emoji: string) => void;
}

export default function IdeaDocument({
	resultSections,
	headingIdxToComments,
	onCommentClick,
	focusedSection,
	sectionReactions,
	onToggleSectionReaction
}: IdeaDocumentProps) {
	const containerRef = useRef<HTMLDivElement | null>(null);

	useEffect(() => {
		// Scroll into view when focusedSection matches outerIndex
		if (containerRef.current && focusedSection !== null) {
			const headingRef = containerRef.current.querySelector(
				`#emojiList-${focusedSection ?? ""}`
			);
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
			{(resultSections ?? []).map((section: ResultSection, outerIndex: number) => {
				const comments = headingIdxToComments[outerIndex] ?? [];
				const canReact = hasContent(section);
				return (
					<div key={outerIndex} id={`heading-${outerIndex}`}>
						<ReactMarkdown>{section.heading}</ReactMarkdown>
						<ReactMarkdown>{section.body}</ReactMarkdown>
						{(canReact || comments.length > 0) && (
							<div
								id={`emojiList-${outerIndex}`}
								className="not-prose flex flex-wrap items-center gap-x-3 gap-y-1"
							>
								{comments.length > 0 && (
									<SectionCommenters
										comments={comments}
										onCommentClick={onCommentClick}
										isFocused={
											focusedSection != null &&
											Number(focusedSection) === outerIndex
										}
									/>
								)}
								{canReact && (
									<ReactionBar
										label={sectionLabel(section, outerIndex)}
										reactions={sectionReactions.filter(
											(r) => r.sectionIndex === outerIndex
										)}
										onToggle={(emoji) =>
											onToggleSectionReaction(outerIndex, emoji)
										}
										note={SECTION_REACTIONS_NOTE}
									/>
								)}
							</div>
						)}
					</div>
				);
			})}
		</div>
	);
}
