import type { IdeaDetail } from "@src/types";
import { useState, useRef, useEffect } from "react";
import { updateIdeaTitleApi, deleteIdeaApi } from "@helpers/api/idea";
import { exportIdeaApi, exportMarkdownApi, shortenHomePath } from "@helpers/api/share";
import { CONFIRM_DELETE_ANNOUNCEMENT, useConfirmClick, useTimeout } from "@src/hooks/useTimeout";
import Button from "@ds/Button";
import { useSnackbar } from "@ds/Snackbar";
import NoteModal from "./NoteModal";

interface IdeaTitleBarProps {
	idea: IdeaDetail;
	isOwnIdea: boolean;
	parentId?: string | null;
	/** refresh the document after a note was appended */
	onNoteAdded?: () => void;
	/** the saved title after a successful rename */
	onTitleSaved?: (savedTitle: string) => void;
	/** switch the summary to the editor */
	onEditSummary?: () => void;
	/** rerun the generation from the stored transcript */
	onRegenerateSummary?: () => void;
	/** a regeneration is running */
	isRegenerating?: boolean;
	/** finished feedback items hanging off this idea: the delete
	 * confirmation names what it cascades to */
	feedbackCount?: number;
}

export default function IdeaTitleBar({
	idea,
	isOwnIdea,
	parentId,
	onNoteAdded,
	onTitleSaved,
	onEditSummary,
	onRegenerateSummary,
	isRegenerating = false,
	feedbackCount = 0
}: IdeaTitleBarProps) {
	const { openSnackbar } = useSnackbar();
	const [exportState, setExportState] = useState<string | null>(null);
	const [isNoteOpen, setIsNoteOpen] = useState(false);
	const {
		isConfirming: confirmingDelete,
		secondsLeft,
		confirm: confirmDelete
	} = useConfirmClick();
	const {
		isConfirming: confirmingRegenerate,
		secondsLeft: regenSeconds,
		confirm: confirmRegenerate
	} = useConfirmClick();
	const [scheduleExportReset] = useTimeout();
	const [isEditing, setIsEditing] = useState(false);
	const [editedTitle, setEditedTitle] = useState(idea.title ?? "");
	// the in-progress value while editing; discarded on Escape
	const [draftTitle, setDraftTitle] = useState(idea.title ?? "");
	const inputRef = useRef<HTMLInputElement | null>(null);
	// Each edit session ends exactly once. Enter/Escape unmount the
	// focused input, and the webview may fire blur during that removal -
	// running onBlur with the previous render's closure, which would
	// commit an Escaped draft or rename twice on Enter.
	const sessionFinishedRef = useRef(true);

	let createdByText = "You";
	if (idea.creatorName) {
		createdByText = idea.creatorName;
	}

	const startEditing = (): void => {
		sessionFinishedRef.current = false;
		setDraftTitle(editedTitle);
		setIsEditing(true);
	};

	/** Claims the end of the current edit session; false if already ended. */
	const endSession = (): boolean => {
		if (sessionFinishedRef.current) return false;
		sessionFinishedRef.current = true;
		return true;
	};

	useEffect(() => {
		if (isEditing) {
			inputRef.current?.focus();
			inputRef.current?.select();
		}
	}, [isEditing]);

	const finishEditing = (): void => {
		if (!endSession()) return;
		const trimmed = draftTitle.trim();
		if (trimmed.length === 0) {
			openSnackbar(false, "Error: Title field is empty");
			setIsEditing(false);
			return;
		}
		if (trimmed === editedTitle) {
			setIsEditing(false);
			return;
		}
		const previous = editedTitle;
		// optimistic: reverted if the save fails
		setEditedTitle(trimmed);
		setIsEditing(false);
		updateIdeaTitleApi(idea.id, trimmed)
			.then(() => onTitleSaved?.(trimmed))
			.catch((err) => {
				console.error("rename failed", err);
				setEditedTitle(previous);
				openSnackbar(false, "Error: Could not save the new title");
			});
	};

	const cancelEditing = (): void => {
		// Escape: discard the draft, keep the saved title
		endSession();
		setIsEditing(false);
	};

	const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>): void => {
		if (e.key === "Enter") {
			e.preventDefault();
			finishEditing();
		} else if (e.key === "Escape") {
			e.preventDefault();
			cancelEditing();
		}
	};

	/** Copy the whole document as Markdown. */
	const handleCopy = () => {
		const text = idea.summary ?? "";
		if (!text.trim()) {
			openSnackbar(false, "Nothing to copy yet");
			return;
		}
		navigator.clipboard
			.writeText(text)
			.then(() => openSnackbar(true, "Copied"))
			.catch((err) => {
				console.error("copy failed", err);
				openSnackbar(false, "Couldn't copy - select the text and copy manually");
			});
	};

	const handleExport = () => {
		setExportState("exporting");
		exportIdeaApi(idea.id)
			.then((res) => {
				if (res.cancelled) {
					setExportState(null);
				} else {
					setExportState("done");
					if (res.path) {
						openSnackbar(true, `Saved to ${shortenHomePath(res.path)}`);
					}
					scheduleExportReset(() => setExportState(null), 4000);
				}
			})
			.catch((err) => {
				console.error("export failed", err);
				setExportState("error");
				scheduleExportReset(() => setExportState(null), 4000);
			});
	};

	/** Export the document (and its transcript) as a local .md file. */
	const [isMarkdownExporting, setIsMarkdownExporting] = useState(false);
	const handleMarkdownExport = () => {
		setIsMarkdownExporting(true);
		exportMarkdownApi(idea.id, true)
			.then((res) => {
				if (!res.cancelled && res.path) {
					openSnackbar(true, `Saved to ${shortenHomePath(res.path)}`);
				}
			})
			.catch((err) => {
				console.error("markdown export failed", err);
				openSnackbar(false, "Couldn't export the Markdown file");
			})
			.finally(() => setIsMarkdownExporting(false));
	};

	const handleDelete = () => {
		if (!confirmDelete()) return;
		deleteIdeaApi(idea.id)
			.then(() => {
				window.location.href = "/dashboard";
			})
			.catch((err) => {
				console.error("delete failed", err);
				openSnackbar(false, "Error: Could not delete this idea");
			});
	};

	const renderActionButtons = () => {
		const buttons = [];
		if (!parentId && !isOwnIdea) {
			// giving feedback is for ideas someone shared with you; your
			// own ideas get notes instead (the feedback interview asks a
			// stranger's questions about your own text)
			buttons.push(
				<Button
					variant="pink"
					key="give-feedback"
					onClick={() => {
						window.location.href = `/chat?parentId=${idea.id}`;
					}}
				>
					Give Feedback
				</Button>
			);
		}
		if (isOwnIdea && !parentId) {
			buttons.push(
				<Button variant="pink" key="add-note" onClick={() => setIsNoteOpen(true)}>
					Add Note
				</Button>
			);
		}
		if (idea.summary !== null && idea.summary !== undefined) {
			buttons.push(
				<Button variant="bordered" key="copy-markdown" onClick={handleCopy}>
					Copy as Markdown
				</Button>
			);
			buttons.push(
				<Button variant="bordered" key="edit-summary" onClick={() => onEditSummary?.()}>
					Edit summary
				</Button>
			);
			buttons.push(
				<Button
					variant="bordered"
					key="regenerate"
					onClick={() => {
						if (confirmRegenerate()) onRegenerateSummary?.();
					}}
					classes={confirmingRegenerate ? "ring-red-400 text-red-600" : ""}
					disabled={isRegenerating}
				>
					{isRegenerating
						? "Regenerating..."
						: confirmingRegenerate
							? `Really regenerate? (${regenSeconds ?? 0}s)`
							: "Regenerate"}
				</Button>
			);
		}
		if (isOwnIdea) {
			buttons.push(
				<Button
					variant="bordered"
					key="export-markdown"
					onClick={handleMarkdownExport}
					disabled={isMarkdownExporting}
				>
					{isMarkdownExporting ? "Exporting..." : "Export as Markdown"}
				</Button>
			);
		}
		if (isOwnIdea) {
			buttons.push(
				<Button variant="pink" key="export" onClick={handleExport}>
					{exportState === "exporting"
						? "Exporting..."
						: exportState === "done"
							? "Exported!"
							: exportState === "error"
								? "Export failed"
								: parentId
									? "Export Feedback"
									: "Export"}
				</Button>
			);
		}
		buttons.push(
			<Button
				variant="bordered"
				key="delete"
				onClick={handleDelete}
				classes={confirmingDelete ? "ring-red-400 text-red-600" : ""}
			>
				{confirmingDelete
					? `Delete idea${feedbackCount > 0 ? ` and ${feedbackCount} feedback item${feedbackCount === 1 ? "" : "s"}?` : "?"} (${secondsLeft ?? 0}s)`
					: "Delete"}
			</Button>,
			// Announced once when armed, outside the button and not on
			// every countdown tick
			<span key="delete-status" role="status" className="sr-only">
				{confirmingDelete ? `${CONFIRM_DELETE_ANNOUNCEMENT} This cannot be undone.` : ""}
			</span>
		);
		return buttons;
	};

	return (
		<div className="px-5 pt-5 md:px-7 md:pt-7">
			<div className="flex flex-wrap gap-4 justify-between">
				<div className="flex flex-wrap gap-4 justify-between">
					{parentId && (
						<a
							href={`/idea?id=${parentId}`}
							className="flex mt-1 h-min"
							title="Go back to original idea"
						>
							<ion-icon
								class="w-5 h-5 hydrated pointer-events-none"
								name="arrow-back-outline"
							></ion-icon>
						</a>
					)}
					<div className="flex flex-col">
						<div className="flex flex-row text-left text-base text-stone-900">
							{!isEditing && (
								<div className="flex flex-row items-center relative group">
									<h1 className="md:mr-2 relative">{editedTitle}</h1>
									<button
										onClick={startEditing}
										aria-label="Rename idea"
										className="disabled:text-stone-400 hover:bg-stone-200 self-center p-1 leading-none rounded-full"
									>
										<ion-icon
											class="w-4 h-4 hydrated pointer-events-none"
											name="create-outline"
											role="img"
										></ion-icon>
									</button>
								</div>
							)}
							{isEditing && (
								<div className="flex flex-row">
									<input
										ref={inputRef}
										aria-label="Idea title"
										value={draftTitle}
										onChange={(e) => setDraftTitle(e.target.value)}
										onKeyDown={handleKeyDown}
										onBlur={finishEditing}
										className="md:mr-4 w-full max-w-xl text-inherit border-b border-stone-300 bg-transparent outline-none focus:border-pink-400"
									/>

									<button
										onClick={finishEditing}
										className="disabled:text-stone-400 hover:bg-stone-200 p-1 leading-none rounded-full"
										aria-label="Finish editing"
									>
										<ion-icon
											class="w-4 h-4 hydrated pointer-events-none"
											name="checkmark-outline"
											role="img"
										></ion-icon>
									</button>
								</div>
							)}
						</div>
						<h2 className="text-sm tracking-tight text-stone-500 mt-1">
							Created by {createdByText}
						</h2>
					</div>
				</div>

				<div className="flex flex-col items-end gap-1 self-center">
					{/* items-center: a stretched row made every button as tall as
					    the tallest item, which used to be Export plus its note */}
					<div className="flex items-center gap-3" role="group">
						{renderActionButtons()}
					</div>
					{/* privacy note: previously buried in a title attribute,
					    invisible to keyboard and touch users */}
					{isOwnIdea && (
						<p className="text-[10px] leading-tight text-stone-500 max-w-[220px] text-right">
							Exports the summary only — the transcript stays on this device.
						</p>
					)}
				</div>
			</div>
			{isNoteOpen && (
				<NoteModal
					ideaId={idea.id}
					onClose={() => setIsNoteOpen(false)}
					onSaved={() => {
						openSnackbar(true, "Note added");
						onNoteAdded?.();
					}}
				/>
			)}
		</div>
	);
}
