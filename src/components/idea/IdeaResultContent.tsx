import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
	getIdeaApi,
	getIdeaChildrenApi,
	markIdeaReadApi,
	setIdeaResultApi
} from "@helpers/api/idea";
import type { IdeaDetail, IdeaFeedbackItem, FeedbackComment } from "@src/types";

import { TailwindComposedTabs } from "@ds/TailwindTabs";
import LoadingAnimation from "@components/global/LoadingAnimation";
import IdeaTitleBar from "./IdeaTitleBar";
import IdeaSummary from "./IdeaSummary";
import IdeaSection from "@components/idea/feedback-aggregation/IdeaSection";
import IdeaTranscript from "./IdeaTranscript";
import IdeaBranches from "./IdeaBranches";
import IdeaNotes from "./IdeaNotes";
import { getQueryParam } from "@helpers/helpers";
import { describeError } from "@helpers/describeError";
import { QUERY_PARAMS } from "@src/tauri/commands";
import {
	continuationChatUrl,
	finishedFeedback,
	isContinuation,
	isOwnIdea,
	parseHeadingIndex,
	stripNotesSection
} from "@helpers/ideas";
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
	const [editingDocument, setEditingDocument] = useState(false);
	const [documentDraft, setDocumentDraft] = useState("");
	const [savingDocument, setSavingDocument] = useState(false);
	const [ideaChildren, setIdeaChildren] = useState<IdeaFeedbackItem[] | null>(null);
	const [continuations, setContinuations] = useState<IdeaFeedbackItem[]>([]);
	const [headingIdxToComments, setHeadingIdxToComments] = useState<
		Record<number, FeedbackComment[]>
	>({});
	// NOTE: there is deliberately no max-width layout branch any more:
	// the window's minWidth is 940px (tauri.conf.json), so a 768px
	// variant that dropped sections and reactions could never render.
	// useMediaQuery stays for reduced-motion checks elsewhere.
	const [isUnread, setIsUnread] = useState(false);
	const { openSnackbar } = useSnackbar();
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
					setContinuations(children.filter(isContinuation));
					const [updateIdeaChildren, updateOidHeadingToFeedbackComments] = mapComments(
						children.filter((child) => !isContinuation(child)),
						res.resultJson ?? []
					);
					setIdeaChildren(updateIdeaChildren);
					setHeadingIdxToComments(updateOidHeadingToFeedbackComments);

					setIsUnread(res.isUnread ?? false);
					const ideaContent: IdeaDetail = {
						id,
						title: res.title,
						type: res.type,
						summary: res.summary,
						transcript: res.transcript,
						isUnread: res.isUnread,
						creatorEmail: res.creatorEmail,
						creatorName: res.creatorName,
						resultJson: res.resultJson,
						notes: res.notes
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

	/** Refresh the document after an out-of-band edit (an added note, a
	 * saved document edit). Silent: the current view stays until the
	 * fresh copy arrives. */
	const reloadIdea = useCallback(
		(
			failureMessage = "The note was saved, but refreshing the page failed - reopen the idea to see it."
		) => {
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
					console.error("failed to refresh the idea after an edit", err);
					openSnackbar(false, failureMessage);
				});
		},
		[ideaId, openSnackbar]
	);

	const copyMarkdown = async () => {
		try {
			await navigator.clipboard.writeText(idea.summary ?? "");
			openSnackbar(true, "Copied as Markdown");
		} catch (err) {
			console.error("copy failed", err);
			openSnackbar(false, "Error: Could not copy Markdown");
		}
	};

	// The backend refuses an edit while feedback, section reactions or
	// notes point into the document (their section indices and quoted
	// spans would drift); hide the button instead of letting the user
	// type an edit that can't be saved. The backend re-checks at save
	// time, inside the write's transaction.
	const editBlocked =
		ideaChildren === null ||
		ideaChildren.length > 0 ||
		sectionReactions.length > 0 ||
		(idea.notes?.length ?? 0) > 0;

	const saveDocument = async () => {
		if (!ideaId || savingDocument || !documentDraft.trim()) return;
		setSavingDocument(true);
		try {
			// the expected previous text makes the backend refuse a save
			// over a document that changed since it was opened
			await setIdeaResultApi(ideaId, documentDraft, idea.summary ?? "");
		} catch (err) {
			console.error("document save failed", err);
			openSnackbar(false, `Error: ${describeError(err).message}`);
			setSavingDocument(false);
			return;
		}
		setEditingDocument(false);
		setSavingDocument(false);
		openSnackbar(true, "Document saved");
		reloadIdea(
			"The document was saved, but refreshing the page failed - reopen the idea to see it."
		);
	};

	// each entry carries its own ?tab= param so the URL values always
	// match the tabs that actually exist (a feedback idea has no feedback
	// tab, and the reverse)
	const notes = idea.notes ?? [];
	// structured notes are shown as the editable list below; the rendered
	// "## Notes" section would duplicate them. Without structured notes
	// (legacy rows), the section stays so nothing is lost.
	const documentSections =
		notes.length > 0
			? (idea.resultJson ?? []).filter(
					(section) => (section.heading ?? "").trim() !== "## Notes"
				)
			: (idea.resultJson ?? []);
	const summaryMarkdown = notes.length > 0 ? stripNotesSection(idea.summary ?? "") : idea.summary;
	const notesBlock =
		!editingDocument && notes.length > 0 && ideaId ? (
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
		parentIdea && !isContinuation(idea)
			? [
					{
						label: "Summary",
						param: "summary",
						content: (
							<>
								{idea.summary?.trim() && (
									<div className="px-7 py-3">
										<button
											type="button"
											className="text-sm underline"
											onClick={() => {
												void copyMarkdown();
											}}
										>
											Copy as Markdown
										</button>
									</div>
								)}
								<IdeaSummary content={summaryMarkdown} />
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
								{idea.summary?.trim() && (
									<div className="flex flex-wrap items-center gap-3 px-7 py-3">
										<button
											type="button"
											className="text-sm underline"
											onClick={() => {
												void copyMarkdown();
											}}
										>
											Copy as Markdown
										</button>
										{isOwn && !editingDocument && !editBlocked && (
											<button
												type="button"
												className="text-sm underline"
												onClick={() => {
													setDocumentDraft(idea.summary ?? "");
													setEditingDocument(true);
												}}
											>
												Edit document
											</button>
										)}
									</div>
								)}
								{editingDocument ? (
									<div className="px-7 pb-7">
										<label
											htmlFor="document-markdown"
											className="block text-sm mb-2"
										>
											Document Markdown
										</label>
										<textarea
											id="document-markdown"
											className="w-full min-h-96 border border-stone-300 rounded p-3 text-sm"
											value={documentDraft}
											onChange={(event) =>
												setDocumentDraft(event.target.value)
											}
											disabled={savingDocument}
										/>
										<p className="text-sm text-stone-500 my-2">
											Editing is unavailable if this idea has feedback,
											reactions, or notes.
										</p>
										<div className="flex gap-3">
											<button
												type="button"
												className="underline text-sm"
												onClick={() => {
													void saveDocument();
												}}
												disabled={savingDocument || !documentDraft.trim()}
											>
												Save changes
											</button>
											<button
												type="button"
												className="underline text-sm"
												onClick={() => setEditingDocument(false)}
												disabled={savingDocument}
											>
												Cancel
											</button>
										</div>
									</div>
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

	if (!parentIdea || isContinuation(idea)) {
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
					feedbackCount={finishedFeedback(ideaChildren ?? []).length}
				/>
				{isContinuation(idea) && parentIdea && (
					<p className="px-7 py-2 text-sm">
						Continued from{" "}
						<a
							className="underline"
							href={`/idea?id=${encodeURIComponent(parentIdea.id)}`}
						>
							{parentIdea.title || "an idea"}
						</a>
					</p>
				)}
				{continuations.length > 0 && (
					<section className="px-7 py-3" aria-label="Continuation sessions">
						<h2 className="font-semibold">Continued thinking</h2>
						<ul>
							{continuations.map((child) => (
								<li key={child.id}>
									<a
										className="underline"
										href={
											child.isDraft
												? continuationChatUrl({
														parentId: ideaId,
														draftId: child.id
													})
												: `/idea?id=${encodeURIComponent(child.id)}`
										}
									>
										{child.isDraft
											? "Resume continuation draft"
											: child.title || "Untitled continuation"}
									</a>
								</li>
							))}
						</ul>
					</section>
				)}
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
