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
	const [headingIdxToComments, setHeadingIdxToComments] = useState<Record<number, FeedbackComment[]>>({});
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

		// legacy behavior: a missing id still fires the API and lands in
		// the error path below
		const id = ideaId as string;
		getIdeaApi(id)
			.then((res) => {
				if (isCurrent) {
					fetchIdeaChildrenData(id).then(
						([updateIdeaChildren, updateOidHeadingToFeedbackComments]) => {
							setIdeaChildren(updateIdeaChildren as IdeaFeedbackItem[]);
							setHeadingIdxToComments(updateOidHeadingToFeedbackComments as Record<number, FeedbackComment[]>);
						}
					);

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
				console.error("PROBABLY IDEA NOT FOUND WITH ID: " + ideaId);
				setErrorFound(true);
				setIsLoading(false);
				throw err;
			});
		return () => {
			isCurrent = false;
			window.removeEventListener("resize", handleResize);
		};
	}, []);

	const tabData: {
		label: string;
		content: ReactNode;
		tooltipText?: string;
		disabled?: boolean;
	}[] =
		isMdSizeOrLess || parentIdea
			? [
					{
						label: "Summary",
						content: <IdeaSummary content={idea.summary} />
					},
					{
						label: "Transcript",
						content: <IdeaTranscript title={idea.title} transcript={idea.transcript} />
					}
			  ]
			: [
					{
						label: "Summary",
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
						content: <IdeaTranscript title={idea.title} transcript={idea.transcript} />
					}
			  ];

	if (!parentIdea) {
		if (ideaChildren && ideaChildren.length > 0) {
			const feedbackCount = ideaChildren.length;
			tabData.push({
				label: feedbackCount > 0 ? `Feedback (${feedbackCount})` : "Feedback",
				content: <IdeaBranches kids={ideaChildren} />
			});
		} else {
			const tooltipText = isOwnIdea
				? "Export your idea and send it to someone for feedback"
				: "No feedback on this idea yet";
			tabData.push({
				label: "Feedback",
				tooltipText: tooltipText,
				disabled: true,
				content: <IdeaBranches kids={[]} />
			});
		}
	}

	if (errorFound) {
		return <ErrorSection title="Idea not found" />;
	} else if (isLoading) {
		return (
			<div className="p-4">
				<LoadingAnimation text="Loading user profile..." />
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
				<TailwindComposedTabs data={tabData} activeTab={activeTab} accentColor="pink" />
			</div>
		);
	}
}

async function fetchIdeaChildrenData(ideaId: string): Promise<[IdeaFeedbackItem[], Record<number, FeedbackComment[]>]> {
	const result = await getIdeaChildrenApi(ideaId)
		.then((res) => {
			const oidHeadingToFeedbackComments: Record<number, FeedbackComment[]> = {};
			const ideaChildren = res.map((idea) => {
				(idea.feedbackComments ?? []).map((comment: FeedbackComment) => {
					const headingIdx = Number((comment.oidHeadingText ?? "").split("#")[0]);
					const currMap: FeedbackComment[] = oidHeadingToFeedbackComments[headingIdx] || [];
					currMap.push({
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
			return [ideaChildren as IdeaFeedbackItem[], oidHeadingToFeedbackComments];
		})
		.catch((err) => {
			console.error("PROBABLY IDEA NOT FOUND WITH ID: " + ideaId, err);
			throw err;
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
				console.error("PROBABLY IDEA NOT FOUND WITH ID: " + id);
				throw err;
			});
		return () => {
			isCurrent = false;
		};
	}, [id]);
}
