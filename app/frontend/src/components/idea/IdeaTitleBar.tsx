import type { IdeaDetail } from "@src/types";
import { useState, useRef, useEffect } from "react";
import { updateIdeaTitleApi, deleteIdeaApi } from "@helpers/api/idea";
import { exportIdeaApi } from "@helpers/api/share";
import { useConfirmClick, useTimeout } from "@src/hooks/useTimeout";
import PinkButton from "@ds/PinkButton";
import BorderedButton from "@ds/BorderedButton";
import { Snackbar } from "@ds/Snackbar";

interface IdeaTitleBarProps {
	idea: IdeaDetail;
	isOwnIdea: boolean;
	isFeedbackMissing?: boolean;
	requestedDraftId?: string;
	parentId?: string | null;
}

export default function IdeaTitleBar({
	idea,
	isOwnIdea,
	requestedDraftId,
	parentId
}: IdeaTitleBarProps) {
	const [snackbarErrorOpen, setSnackbarErrorOpen] = useState(false);
	const [snackbarErrorMessage, setSnackbarErrorMessage] = useState("Error: Title field is empty");
	const [exportState, setExportState] = useState<string | null>(null);
	const { isConfirming: confirmingDelete, confirm: confirmDelete } = useConfirmClick();
	const [scheduleExportReset] = useTimeout();
	const [isEditing, setIsEditing] = useState(false);
	const [editedTitle, setEditedTitle] = useState(idea.title ?? "");
	// the in-progress value while editing; discarded on Escape
	const [draftTitle, setDraftTitle] = useState(idea.title ?? "");
	const inputRef = useRef<HTMLInputElement | null>(null);

	let createdByText = "You";
	if (idea.creatorName) {
		createdByText = idea.creatorName;
	}

	const startEditing = (): void => {
		setDraftTitle(editedTitle);
		setIsEditing(true);
	};

	useEffect(() => {
		if (isEditing) {
			inputRef.current?.focus();
			inputRef.current?.select();
		}
	}, [isEditing]);

	const finishEditing = (): void => {
		const trimmed = draftTitle.trim();
		if (trimmed.length === 0) {
			setSnackbarErrorMessage("Error: Title field is empty");
			setSnackbarErrorOpen(true);
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
		updateIdeaTitleApi(idea.id, trimmed).catch((err) => {
			console.error("rename failed", err);
			setEditedTitle(previous);
			setSnackbarErrorMessage("Error: Could not save the new title");
			setSnackbarErrorOpen(true);
		});
	};

	const cancelEditing = (): void => {
		// Escape: discard the draft, keep the saved title
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

	const handleExport = () => {
		setExportState("exporting");
		exportIdeaApi(idea.id)
			.then((res) => {
				if (res.cancelled) {
					setExportState(null);
				} else {
					setExportState("done");
					scheduleExportReset(() => setExportState(null), 4000);
				}
			})
			.catch((err) => {
				console.log("export failed", err);
				setExportState("error");
				scheduleExportReset(() => setExportState(null), 4000);
			});
	};

	const handleDelete = () => {
		if (!confirmDelete()) return;
		deleteIdeaApi(idea.id)
			.then(() => {
				window.location.href = "/dashboard";
			})
			.catch((err) => {
				console.log("delete failed", err);
				setSnackbarErrorMessage("Error: Could not delete this idea");
				setSnackbarErrorOpen(true);
			});
	};

	const renderActionButtons = () => {
		const buttons = [];
		if (!parentId) {
			buttons.push(
				<PinkButton
					key="give-feedback"
					onClick={() => {
						const feedbackHref = requestedDraftId
							? `/chat?id=${requestedDraftId}`
							: `/chat?parentId=${idea.id}`;
						window.location.href = feedbackHref;
					}}
				>
					Give Feedback
				</PinkButton>
			);
		}
		if (isOwnIdea) {
			buttons.push(
				<span key="export-wrap" className="flex flex-col items-end">
					<PinkButton onClick={handleExport}>
						{exportState === "exporting"
							? "Exporting..."
							: exportState === "done"
								? "Exported!"
								: exportState === "error"
									? "Export failed"
									: parentId
										? "Export Feedback"
										: "Export"}
					</PinkButton>
					{/* privacy note: previously buried in a title attribute,
					    invisible to keyboard and touch users */}
					<p className="text-[10px] leading-tight text-stone-500 mt-1 max-w-[220px] text-right">
						Exports the summary only — the transcript stays on this device.
					</p>
				</span>
			);
		}
		buttons.push(
			<BorderedButton
				key="delete"
				onClick={handleDelete}
				classes={confirmingDelete ? "border-red-400 text-red-600" : ""}
			>
				{confirmingDelete ? "Really delete?" : "Delete"}
			</BorderedButton>
		);
		return buttons;
	};

	return (
		<div className="px-5 pt-5 md:px-7 md:pt-7">
			{snackbarErrorOpen && (
				<Snackbar
					isSuccess={false}
					message={snackbarErrorMessage}
					onClose={() => setSnackbarErrorOpen(false)}
				/>
			)}
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

				<div className="flex gap-3 rounded-md shadow-sm self-center" role="group">
					{renderActionButtons()}
				</div>
			</div>
		</div>
	);
}
