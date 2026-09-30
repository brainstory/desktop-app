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
import ErrorSection from "../error/ErrorSection";

export default function IdeaResultContent() {
	const tabs: Record<string, number> = {
		summary: 0,
		transcript: 1,
		feedback: 2
	};

	const ideaId = getQueryParam("id") ?? undefined;
	const [activeTab] = useState<number>(() => {
		const tab = getQueryParam("tab");
		return tab ? (tabs[tab] ?? 0) : tabs.summary;
	});
	const [isLoading, setIsLoading] = useState(true);
	const [idea, setIdea] = useState<IdeaDetail>({ id: "" });
	const [parentIdea, setParentIdea] = useState<IdeaDetail["parentIdea"]>(null);
	const [errorFound, setErrorFound] = useState(false);
	const [ideaChildren, setIdeaChildren] = useState<IdeaFeedbackItem[] | null>(null);
	const [headingIdxToComments, setHeadingIdxToComments] = useState<
		Record<number, FeedbackComment[]>
	>({});
	const [isMdSizeOrLess, setIsMdSizeOrLess] = useState(window.innerWidth <= 768);
	const [isUnread, setIsUnread] = useState(false);

	// Opening an unread (imported feedback) idea marks it read.
	useMarkReadApi(ideaId, isUnread);

	// own ideas have no creator attribution; imported ones carry creator info
	const isOwnIdea = !idea.creatorEmail && !idea.creatorName;

	useEffect(() => {
		// Event listener to keep track of screen size
		const handleResize = () => {
			setIsMdSizeOrLess(window.innerWidth <= 768);
		};
		window.addEventListener("resize", handleResize);

		let isCurrent = true;

		// no ?id in the URL: there is nothing to load (rendered as an
		// error section below instead of invoking the API with undefined)
		const id = ideaId ?? "";
		if (!id) {
			return () => {
				isCurrent = false;
				window.removeEventListener("resize", handleResize);
			};
		}
		getIdeaApi(id)
			.then((res) => {
				if (isCurrent) {
					fetchIdeaChildrenData(id)
						.then(([updateIdeaChildren, updateOidHeadingToFeedbackComments]) => {
							if (!isCurrent) return;
							setIdeaChildren(updateIdeaChildren as IdeaFeedbackItem[]);
							setHeadingIdxToComments(
								updateOidHeadingToFeedbackComments as Record<
									number,
									FeedbackComment[]
								>
							);
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
						numOfShares: res.sharedWithUsers?.length ?? 0,
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
			window.removeEventListener("resize", handleResize);
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
						content: <IdeaTranscript title={idea.title} transcript={idea.transcript} />
					}
				]
			: [
					{
						label: "Summary",
						param: "summary",
						content: (
							<IdeaSection
								resultSections={idea.resultJson ?? []}
								ideaFeedbackChildren={ideaChildren}
								headingIdxToComments={headingIdxToComments}
								canShare={isOwnIdea}
							/>
						)
					},
					{
						label: "Transcript",
						param: "transcript",
						content: <IdeaTranscript title={idea.title} transcript={idea.transcript} />
					}
				];

	if (!parentIdea) {
		if (ideaChildren && ideaChildren.length > 0) {
			const feedbackCount = ideaChildren.length;
			tabData.push({
				label: feedbackCount > 0 ? `Feedback (${feedbackCount})` : "Feedback",
				param: "feedback",
				content: <IdeaBranches kids={ideaChildren} />
			});
		} else {
			const tooltipText = isOwnIdea
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
				<IdeaTitleBar
					idea={idea}
					isOwnIdea={isOwnIdea}
					isFeedbackMissing={!parentIdea}
					parentId={parentIdea?.id}
				/>
				<TailwindComposedTabs
					data={tabData}
					activeTab={activeTab}
					accentColor="pink"
					tabParams={tabData.map((tab) => tab.param)}
				/>
			</div>
		);
	}
}

async function fetchIdeaChildrenData(
	ideaId: string
): Promise<[IdeaFeedbackItem[], Record<number, FeedbackComment[]>]> {
	const result = await getIdeaChildrenApi(ideaId).then((res) => {
		const oidHeadingToFeedbackComments: Record<number, FeedbackComment[]> = {};
		const ideaChildren = res.map((idea) => {
			(idea.feedbackComments ?? []).forEach((comment: FeedbackComment) => {
				const headingIdx = Number((comment.oidHeadingText ?? "").split("#")[0]);
				if (!Number.isInteger(headingIdx)) {
					// a comment whose heading reference can't be parsed can
					// never be attached to a section; keep it out loudly
					// instead of filing it under a silent NaN key
					console.warn(
						"feedback comment with unparsable heading reference:",
						comment.oidHeadingText
					);
					return;
				}
				const currMap: FeedbackComment[] = oidHeadingToFeedbackComments[headingIdx] || [];
				currMap.push({
					commentId: `${idea.id}:${headingIdx}:${currMap.length}`,
					ideaId: idea.id,
					creatorEmail: idea.creatorEmail,
					creatorName: idea.creatorName,
					createdAt: idea.createdAt,
					matchedSpans: comment.matchedSpans,
					feedbackText: comment.feedbackText,
					labels: comment.labels
				});
				oidHeadingToFeedbackComments[headingIdx] = currMap;
			});
			return { ...idea, isFeedback: true };
		});
		return [ideaChildren as IdeaFeedbackItem[], oidHeadingToFeedbackComments] as const;
	});

	return result as [IdeaFeedbackItem[], Record<number, FeedbackComment[]>];
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
					console.log("marked idea as read: ", res);
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
