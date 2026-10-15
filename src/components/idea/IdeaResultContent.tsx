import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
	getIdeaApi,
	getIdeaChildrenApi,
	markIdeaReadApi,
	regenerateResultApi,
	setIdeaResultApi
} from "@helpers/api/idea";
import type { IdeaDetail, IdeaFeedbackItem, FeedbackComment } from "@src/types";

import { TailwindComposedTabs } from "@ds/TailwindTabs";
import Button from "@ds/Button";
import LoadingAnimation from "@components/global/LoadingAnimation";
import IdeaTitleBar from "./IdeaTitleBar";
import IdeaSummary from "./IdeaSummary";
import IdeaSection from "@components/idea/feedback-aggregation/IdeaSection";
import IdeaTranscript from "./IdeaTranscript";
import IdeaBranches from "./IdeaBranches";
import IdeaNotes from "./IdeaNotes";
import { getQueryParam, normalizeApiError } from "@helpers/helpers";
import { QUERY_PARAMS } from "@src/tauri/commands";
import { finishedFeedback, isOwnIdea, parseHeadingIndex, stripNotesSection } from "@helpers/ideas";
import ErrorSection from "../error/ErrorSection";
import ErrorBoundary from "@components/global/ErrorBoundary";
import { SnackbarProvider, useSnackbar } from "@ds/Snackbar";
import { useIdeaReactions } from "./reactions/useIdeaReactions";

function IdeaResultContentInner() {
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
	// NOTE: there is deliberately no max-width layout branch any more:
	// the window's minWidth is 940px (tauri.conf.json), so a 768px
	// variant that dropped sections and reactions could never render.
	// useMediaQuery stays for reduced-motion checks elsewhere.
	const [isUnread, setIsUnread] = useState(false);
	const { openSnackbar } = useSnackbar();
	// manual summary editing
	const [isEditingSummary, setIsEditingSummary] = useState(false);
	const [summaryDraft, setSummaryDraft] = useState("");
	const [isSavingSummary, setIsSavingSummary] = useState(false);
	// regeneration
	const [isRegenerating, setIsRegenerating] = useState(false);
	const [regenStatus, setRegenStatus] = useState<string | null>(null);
	const [regenText, setRegenText] = useState("");
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
		// the idea and its children load in parallel: the children only
		// need the idea's sections for comment mapping, which runs after
		// both arrive
		Promise.all([
			getIdeaApi(id),
			getIdeaChildrenApi(id).catch((err) => {
				// feedback children are auxiliary: the page still renders
				// without them
				console.error("failed to load feedback for idea", id, err);
				return [];
			})
		])
			.then(([res, children]) => {
				if (isCurrent) {
					const [updateIdeaChildren, updateOidHeadingToFeedbackComments] = mapComments(
						children,
						res.resultJson ?? []
					);
					setIdeaChildren(updateIdeaChildren);
					setHeadingIdxToComments(updateOidHeadingToFeedbackComments);

					setIsUnread(res.isUnread ?? false);
					const ideaContent: IdeaDetail = {
						id,
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

					setIdea(ideaContent);
					// the window title follows the document, not the app
					document.title = `${ideaContent.title ?? "Idea"} | Brainstory`;
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

	/** A rename landed: mirror it into the window title without a
	 * refetch (the title bar already holds the saved value). */
	const handleTitleSaved = useCallback((savedTitle: string) => {
		document.title = `${savedTitle || "Idea"} | Brainstory`;
	}, []);

	/** Refresh the document after an out-of-band edit (an added note).
	 * Silent: the current view stays until the fresh copy arrives. */
	const reloadIdea = useCallback(() => {
		const id = ideaId ?? "";
		if (!id) return;
		getIdeaApi(id)
			.then((res) => {
				setIdea((prev) => ({
					...prev,
					title: res.title,
					summary: res.summary,
					transcript: res.transcript,
					resultJson: res.resultJson,
					isUnread: res.isUnread,
					creatorEmail: res.creatorEmail,
					creatorName: res.creatorName,
					notes: res.notes
				}));
			})
			.catch((err) => {
				console.error("failed to refresh the idea after a note was added", err);
				openSnackbar(
					false,
					"The note was saved, but refreshing the page failed - reopen the idea to see it."
				);
			});
	}, [ideaId, openSnackbar]);

	// each entry carries its own ?tab= param so the URL values always
	// match the tabs that actually exist (a feedback idea has no feedback
	// tab, and the reverse)
	const notes = idea.notes ?? [];
	// structured notes are shown as the editable list below; the rendered
	// "## Notes" section would duplicate them. Without structured notes
	// (legacy rows), the section stays so nothing is lost.
	const startEditingSummary = (): void => {
		setSummaryDraft(summaryMarkdown ?? "");
		setIsEditingSummary(true);
	};

	const saveSummaryEdit = (): void => {
		const trimmed = summaryDraft.trim();
		if (!trimmed || !ideaId || trimmed === (summaryMarkdown ?? "")) {
			setIsEditingSummary(false);
			return;
		}
		setIsSavingSummary(true);
		setIdeaResultApi(ideaId, trimmed)
			.then(() => {
				setIdea((prev) => ({ ...prev, summary: trimmed }));
				setIsEditingSummary(false);
				openSnackbar(true, "Summary updated");
				// the sectioned view derives from result_json: refetch so
				// both views show the edited text
				reloadIdea();
			})
			.catch((err) => {
				console.error("summary edit failed", err);
				openSnackbar(false, normalizeApiError(err));
			})
			.finally(() => setIsSavingSummary(false));
	};

	const regenerateSummary = (): void => {
		if (!ideaId) return;
		setIsRegenerating(true);
		setRegenText("");
		setRegenStatus(null);
		const { channel, invokePromise } = regenerateResultApi(ideaId);
		channel.onmessage = (raw: unknown) => {
			const event = raw as { type: string; content?: string };
			if (event.type === "chunk" && typeof event.content === "string") {
				setRegenText((prev) => prev + event.content);
			} else if (event.type === "cumulative" && typeof event.content === "string") {
				setRegenText(event.content);
			} else if (event.type === "status" && typeof event.content === "string") {
				setRegenStatus(event.content);
			}
		};
		invokePromise
			.then((res) => {
				const text = res.response ?? "";
				if (text) {
					setIdea((prev) => ({ ...prev, summary: text }));
				}
				setIsRegenerating(false);
				openSnackbar(
					true,
					"Summary regenerated - the previous one is kept in the idea metadata"
				);
			})
			.catch((err) => {
				console.error("regeneration failed", err);
				setIsRegenerating(false);
				openSnackbar(false, normalizeApiError(err));
			});
	};

	/** shared by both summary-tab layouts */
	const regenerationBlock = (
		<div className="py-8 text-center">
			<p className="text-stone-700 font-medium mb-2">
				{regenStatus ?? "Regenerating your summary..."}
			</p>
			{regenText && (
				<p className="text-sm text-stone-500 whitespace-pre-wrap font-mono max-h-64 overflow-y-auto text-left">
					{regenText}
				</p>
			)}
		</div>
	);
	const summaryEditorBlock = (
		<div className="flex flex-col gap-2">
			<textarea
				aria-label="Edit summary"
				rows={16}
				value={summaryDraft}
				onChange={(e) => setSummaryDraft(e.target.value)}
				onKeyDown={(e) => {
					if (e.key === "Escape") {
						e.preventDefault();
						setIsEditingSummary(false);
					} else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
						e.preventDefault();
						saveSummaryEdit();
					}
				}}
				className="w-full text-sm px-4 py-2 border border-stone-200 rounded-md focus:outline-none focus:border-accent-500 resize-y bg-white text-stone-900 font-mono"
			/>
			<div className="flex justify-end gap-2">
				<Button variant="transparent" onClick={() => setIsEditingSummary(false)}>
					Cancel
				</Button>
				<Button
					variant="pink"
					disabled={isSavingSummary || summaryDraft.trim() === ""}
					onClick={saveSummaryEdit}
				>
					{isSavingSummary ? "Saving..." : "Save"}
				</Button>
			</div>
		</div>
	);

	const documentSections =
		notes.length > 0
			? (idea.resultJson ?? []).filter(
					(section) => (section.heading ?? "").trim() !== "## Notes"
				)
			: (idea.resultJson ?? []);
	const summaryMarkdown = notes.length > 0 ? stripNotesSection(idea.summary ?? "") : idea.summary;
	const notesBlock =
		notes.length > 0 && ideaId ? (
			<IdeaNotes ideaId={ideaId} notes={notes} onChanged={reloadIdea} />
		) : null;
	const tabData: {
		label: string;
		param: string;
		content: ReactNode;
		tooltipText?: string;
		disabled?: boolean;
	}[] =
		// feedback pages use the compact two-pane layout; regular ideas
		// the sectioned three-pane one
		parentIdea
			? [
					{
						label: "Summary",
						param: "summary",
						content: (
							<>
								{isRegenerating ? (
									regenerationBlock
								) : isEditingSummary ? (
									summaryEditorBlock
								) : (
									<IdeaSummary content={summaryMarkdown} />
								)}
								{notesBlock}
							</>
						)
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
							<>
								{isRegenerating ? (
									regenerationBlock
								) : isEditingSummary ? (
									summaryEditorBlock
								) : (
									<IdeaSection
										resultSections={documentSections}
										headingIdxToComments={headingIdxToComments}
										sectionReactions={sectionReactions}
										onToggleSectionReaction={toggleSectionReaction}
										commentReactions={commentReactions}
										onToggleCommentReaction={toggleCommentReaction}
									/>
								)}
								{notesBlock}
							</>
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
			tabData.push({
				label: "Feedback",
				param: "feedback",
				tooltipText: isOwn ? undefined : "No feedback on this idea yet",
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
					isOwnIdea={isOwn}
					parentId={parentIdea?.id}
					onNoteAdded={reloadIdea}
					onTitleSaved={handleTitleSaved}
					onEditSummary={startEditingSummary}
					onRegenerateSummary={regenerateSummary}
					isRegenerating={isRegenerating}
					feedbackCount={finishedFeedback(ideaChildren ?? []).length}
				/>
				<TailwindComposedTabs
					data={tabData}
					activeTab={activeTab}
					tabParams={tabData.map((tab) => tab.param)}
				/>
			</div>
		);
	}
}

function mapComments(
	res: IdeaFeedbackItem[],
	sections: { heading?: string | null }[]
): [IdeaFeedbackItem[], Record<number, FeedbackComment[]>] {
	{
		const result = ((): [IdeaFeedbackItem[], Record<number, FeedbackComment[]>] => {
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
		})();
		return result;
	}
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

/** The idea page as an island root, behind a crash boundary. */
export default function IdeaResultContent() {
	return (
		<SnackbarProvider>
			<ErrorBoundary title="This idea page hit a snag">
				<IdeaResultContentInner />
			</ErrorBoundary>
		</SnackbarProvider>
	);
}
