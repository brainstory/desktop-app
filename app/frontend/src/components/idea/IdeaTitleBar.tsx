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
	const [editedTitle, setEditedTitle] = useState(idea.title);
	const textarea = useRef<HTMLHeadingElement | null>(null);

	let createdByText = "You";
	if (idea.creatorName) {
		createdByText = idea.creatorName;
	}

	const startEditing = (): void => {
		setIsEditing(true);
		textarea.current?.focus();
	};

	const finishEditing = (): void => {
		const strippedTitle = (textarea.current?.innerText ?? "").replace(/\n/g, "").trim();
		if (strippedTitle.length === 0) {
			setSnackbarErrorOpen(true);
		} else {
			setEditedTitle(strippedTitle);
			updateIdeaTitleApi(idea.id, strippedTitle).catch((err) => {
				console.log("rename failed", err);
				setSnackbarErrorMessage("Error: Could not save the new title");
				setSnackbarErrorOpen(true);
			});
		}

		setIsEditing(false);
	};

	useEffect(() => {
		if (isEditing && textarea.current) {
			textarea.current.focus();
		}
	}, [isEditing]);

	const handleKeyDown = (e: React.KeyboardEvent): void => {
		if (e.key === "Enter") {
			finishEditing();
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
				<PinkButton
					key="export"
					onClick={handleExport}
					title="Exports the idea summary only — the conversation transcript stays on this device"
				>
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
									<button onClick={startEditing}>
										<h1
											className="group-hover:underline underline-offset-2 decoration-dotted rounded cursor-text md:mr-2 relative"
											title="Rename"
										>
											{editedTitle}
										</h1>
									</button>
									<button
										onClick={startEditing}
										aria-label="Start editing"
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
									<h1
										tabIndex={0}
										ref={textarea}
										role="textbox"
										contentEditable="true"
										onKeyDown={handleKeyDown}
										onFocus={() => {
											// set cursor to end when focused
											if (!textarea.current) return;
											const range = document.createRange();
											const sel = window.getSelection();
											range.selectNodeContents(textarea.current);
											range.collapse(false);
											sel?.removeAllRanges();
											sel?.addRange(range);
										}}
										autoFocus
										className={`md:mr-4 resize-none border-none bg-transparent outline-none focus:ring-0`}
									>
										{editedTitle}
									</h1>

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
