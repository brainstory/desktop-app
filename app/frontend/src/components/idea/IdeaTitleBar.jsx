import { useState, useRef, useEffect } from "react";
import { updateIdeaTitleApi, deleteIdeaApi } from "@helpers/api/idea";
import { exportIdeaApi } from "@helpers/api/share";
import PinkButton from "@ds/PinkButton.jsx";
import BorderedButton from "@ds/BorderedButton.jsx";

export default function IdeaTitleBar({
	idea,
	isOwnIdea,
	requestedDraftId,
	parentId
}) {
	const [snackbarErrorOpen, setSnackbarErrorOpen] = useState(false);
	const [exportState, setExportState] = useState(null);
	const [confirmingDelete, setConfirmingDelete] = useState(false);
	const [isEditing, setIsEditing] = useState(false);
	const [editedTitle, setEditedTitle] = useState(idea.title);
	const textarea = useRef(null);

	let createdByText = "You";
	if (idea.creatorName) {
		createdByText = idea.creatorName;
	}

	const startEditing = () => {
		setIsEditing(true);
		if (textarea.current) {
			textarea.current.focus();
		}
	};

	const finishEditing = () => {
		const strippedTitle = textarea.current.innerText.replace(/\n/g, "").trim();
		if (strippedTitle.length === 0) {
			setSnackbarErrorOpen(true);
			setTimeout(() => {
				setSnackbarErrorOpen(false);
			}, 4000);
		} else {
			setEditedTitle(strippedTitle);
			updateIdeaTitleApi(idea.id, strippedTitle).then((res) => {
				console.log("SAVED", res);
			});
		}

		setIsEditing(false);
	};

	useEffect(() => {
		if (isEditing && textarea.current) {
			textarea.current.focus();
		}
	}, [isEditing]);

	const handleKeyDown = (e) => {
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
					setTimeout(() => setExportState(null), 4000);
				}
			})
			.catch((err) => {
				console.log("export failed", err);
				setExportState("error");
				setTimeout(() => setExportState(null), 4000);
			});
	};

	const handleDelete = () => {
		if (!confirmingDelete) {
			setConfirmingDelete(true);
			// require a fresh confirmation click; reset if they wander off
			setTimeout(() => setConfirmingDelete(false), 5000);
			return;
		}
		deleteIdeaApi(idea.id)
			.then(() => {
				window.location.href = "/dashboard";
			})
			.catch((err) => {
				console.log("delete failed", err);
				setConfirmingDelete(false);
			});
	};

	const renderActionButtons = () => {
		const buttons = [];
		if (!parentId) {
			buttons.push(
				<PinkButton
					key="give-feedback"
					onClick={() => {
						let feedbackHref = requestedDraftId
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
				<div className="mt-3 z-50 bg-red-500 p-4 rounded-md shadow-lg absolute left-1/2 -translate-x-1/2 -translate-y-1/2 text-center min-w-[300px]">
					<p className="text-white">Error: Title field is empty</p>
				</div>
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
										tabIndex="0"
										ref={textarea}
										role="textbox"
										contentEditable="true"
										onKeyDown={handleKeyDown}
										onFocus={() => {
											// set cursor to end when focused
											const range = document.createRange();
											const sel = window.getSelection();
											range.selectNodeContents(textarea.current);
											range.collapse(false);
											sel.removeAllRanges();
											sel.addRange(range);
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
