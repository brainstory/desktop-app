import { useEffect, useState, type ReactNode } from "react";
import { getIdeaApi, getIdeaChildrenApi, markIdeaReadApi } from "@helpers/api/idea";
import type { IdeaDetail, IdeaFeedbackItem, FeedbackComment } from "@src/types";

import { TailwindComposedTabs } from "@ds/TailwindTabs";
import LoadingAnimation from "@components/global/LoadingAnimation";
import IdeaTitleBar from "./IdeaTitleBar";
import IdeaSummary from "./IdeaSummary";
import IdeaSection from "@components/idea/feedback-aggregation/IdeaSection";
import IdeaTranscript from "./IdeaTranscript";
import IdeaBranches from "./IdeaBranches";
import { getQueryParam } from "@helpers/helpers";
import { QUERY_PARAMS } from "@src/tauri/commands";
import { finishedFeedback, isOwnIdea, parseHeadingIndex } from "@helpers/ideas";
import { useMediaQuery } from "@src/hooks/useMediaQuery";
import ErrorSection from "../error/ErrorSection";
import { useSnackbar } from "@ds/Snackbar";
import { useIdeaReactions } from "./reactions/useIdeaReactions";

export default function IdeaResultContent() {
	const tabs: Record<string, number> = {
		summary: 0,
		transcript: 1,
		feedback: 2
	};

	const ideaId = getQueryParam(QUERY_PARAMS.id) ?? undefined;
	const [activeTab] = useState<number>(() => {
		const tab = getQueryParam(QUERY_PARAMS.tab);
		return tab ? (tabs[tab] ?? 0) : tabs.summary!;
	});
	const [isLoading, setIsLoading] = useState(true);
	const [idea, setIdea] = useState<IdeaDetail>({ id: "" });
	const [parentIdea, setParentIdea] = useState<IdeaDetail["parentIdea"]>(null);
	const [errorFound, setErrorFound] = useState(false);
	const [ideaChildren, setIdeaChildren] = useState<IdeaFeedbackItem[] | null>(null);
	const [headingIdxToComments, setHeadingIdxToComments] = useState<
		Record<number, FeedbackComment[]>
	>({});
	// one matchMedia listener that only fires when the 768px boundary is
	// crossed (not a re-render on every resize event)
	const isMdSizeOrLess = useMediaQuery("(max-width: 768px)");
	const [isUnread, setIsUnread] = useState(false);
	const { openSnackbar, snackbars } = useSnackbar();
	const { sectionReactions, toggleSectionReaction, commentReactions, toggleCommentReaction } =
		useIdeaReactions(ideaId, (message) => openSnackbar(false, message));

	// Opening an unread (imported feedback) idea marks it read.
	useMarkReadApi(ideaId, isUnread);

	// own ideas have no creator attribution (imports carry creatorName)
	const isOwn = isOwnIdea(idea);

	useEffect(() => {
		let isCurrent = true;

		// no ?id in the URL: there is nothing to load (rendered as an
		// error section below instead of invoking the API with undefined)
		const id = ideaId ?? "";
		if (!id) {
			return () => {
				isCurrent = false;
			};
		}
		getIdeaApi(id)
			.then((res) => {
				if (isCurrent) {
					fetchIdeaChildrenData(id, res.resultJson ?? [])
						.then(([updateIdeaChildren, updateOidHeadingToFeedbackComments]) => {
							if (!isCurrent) return;
							setIdeaChildren(updateIdeaChildren);
							setHeadingIdxToComments(updateOidHeadingToFeedbackComments);
						})
						.catch((err) => {
							// feedback children are auxiliary: the page still
							// renders without them
							console.error("failed to load feedback for idea", id, err);
							if (isCurrent) setIdeaChildren([]);
						});

					setIsUnread(res.isUnread ?? false);
					const ideaContent = {
						id: ideaId,
						title: res.title,
						summary: res.summary,
						transcript: res.transcript,
						isUnread: res.isUnread,
						creatorEmail: res.creatorEmail,
						creatorName: res.creatorName,
						resultJson: res.resultJson
					};

					if (res?.parentIdea) {
						setParentIdea(res.parentIdea);
					}

					setIdea(ideaContent as IdeaDetail);
					setIsLoading(false);
				}
			})
			.catch((err) => {
				// no rethrow: this catch is the end of the chain, and a
				// floating rejection would fire on every failed load
				console.error("PROBABLY IDEA NOT FOUND WITH ID", ideaId, err);
				if (isCurrent) {
					setErrorFound(true);
					setIsLoading(false);
				}
			});
		return () => {
			isCurrent = false;
		};
	}, [ideaId]);

	// each entry carries its own ?tab= param so the URL values always
	// match the tabs that actually exist (a feedback idea has no feedback
	// tab, and the reverse)
	const tabData: {
		label: string;
		param: string;
		content: ReactNode;
		tooltipText?: string;
		disabled?: boolean;
	}[] =
		isMdSizeOrLess || parentIdea
			? [
					{
						label: "Summary",
						param: "summary",
						content: <IdeaSummary content={idea.summary} />
					},
					{
						label: "Transcript",
						param: "transcript",
						content: <IdeaTranscript transcript={idea.transcript} />
					}
				]
			: [
					{
						label: "Summary",
						param: "summary",
						content: (
							<IdeaSection
								resultSections={idea.resultJson ?? []}
								headingIdxToComments={headingIdxToComments}
								canShare={isOwn}
								sectionReactions={sectionReactions}
								onToggleSectionReaction={toggleSectionReaction}
								commentReactions={commentReactions}
								onToggleCommentReaction={toggleCommentReaction}
							/>
						)
					},
					{
						label: "Transcript",
						param: "transcript",
						content: <IdeaTranscript transcript={idea.transcript} />
					}
				];

	if (!parentIdea) {
		// The tab opens for any child, an unfinished draft included (the
		// tab is where that draft is resumed from), but the count is only
		// the finished feedback.
		if (ideaChildren && ideaChildren.length > 0) {
			const feedbackCount = finishedFeedback(ideaChildren).length;
			tabData.push({
				label: feedbackCount > 0 ? `Feedback (${feedbackCount})` : "Feedback",
				param: "feedback",
				content: (
					<IdeaBranches
						kids={ideaChildren}
						parentId={ideaId}
						onDraftDeleted={(deletedId) =>
							setIdeaChildren(
								(prev) => prev?.filter((child) => child.id !== deletedId) ?? prev
							)
						}
					/>
				)
			});
		} else {
			const tooltipText = isOwn
				? "Export your idea and send it to someone for feedback"
				: "No feedback on this idea yet";
			tabData.push({
				label: "Feedback",
				param: "feedback",
				tooltipText: tooltipText,
				disabled: true,
				content: <IdeaBranches kids={[]} />
			});
		}
	}

	if (!ideaId) {
		return (
			<ErrorSection
				title="No idea selected"
				paragraphs={["Open an idea from your library."]}
			/>
		);
	}
	if (errorFound) {
		return <ErrorSection title="Idea not found" />;
	} else if (isLoading) {
		return (
			<div className="p-4">
				<LoadingAnimation text="Loading idea..." />
			</div>
		);
	} else {
		return (
			<div className="h-full">
				{snackbars}
				<IdeaTitleBar idea={idea} isOwnIdea={isOwn} parentId={parentIdea?.id} />
				<TailwindComposedTabs
					data={tabData}
					activeTab={activeTab}
					tabParams={tabData.map((tab) => tab.param)}
				/>
			</div>
		);
	}
}

async function fetchIdeaChildrenData(
	ideaId: string,
	sections: { heading?: string | null }[]
): Promise<[IdeaFeedbackItem[], Record<number, FeedbackComment[]>]> {
	const result = await getIdeaChildrenApi(ideaId).then(
		(res): [IdeaFeedbackItem[], Record<number, FeedbackComment[]>] => {
			const oidHeadingToFeedbackComments: Record<number, FeedbackComment[]> = {};
			const ideaChildren = res.map((idea) => {
				(idea.feedbackComments ?? []).forEach((comment: FeedbackComment) => {
					const headingIdx = parseHeadingIndex(comment.oidHeadingText, sections);
					if (headingIdx === null) {
						// a comment whose heading reference is empty, not a
						// positive integer, or past the last heading can never be
						// attached to a section; keep it out loudly instead of
						// filing it under heading 0 / a NaN key
						console.warn(
							"feedback comment with an invalid heading reference:",
							comment.oidHeadingText
						);
						return;
					}
					const currMap: FeedbackComment[] =
						oidHeadingToFeedbackComments[headingIdx] || [];
					currMap.push({
						commentId: `${idea.id}:${headingIdx}:${currMap.length}`,
						ideaId: idea.id,
						creatorEmail: idea.creatorEmail,
						creatorName: idea.creatorName,
						createdAt: idea.createdAt,
						matchedSpans: comment.matchedSpans,
						feedbackText: comment.feedbackText,
						itemIndex: comment.itemIndex
					});
					oidHeadingToFeedbackComments[headingIdx] = currMap;
				});
				return { ...idea, isFeedback: true };
			});
			return [ideaChildren, oidHeadingToFeedbackComments];
		}
	);

	return result;
}

function useMarkReadApi(id: string | undefined, isUnread: boolean): void {
	useEffect(() => {
		let isCurrent = true;

		if (!isUnread) {
			return;
		}

		if (id === undefined) return;
		markIdeaReadApi(id)
			.then((res) => {
				if (isCurrent) {
					console.error("marked idea as read: ", res);
				}
			})
			.catch((err) => {
				// best-effort flag: failing to mark read must not surface
				// as an unhandled rejection
				console.error("failed to mark idea as read", id, err);
			});
		return () => {
			isCurrent = false;
		};
	}, [id, isUnread]);
}
