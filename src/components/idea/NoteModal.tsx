import { useEffect, useRef, useState } from "react";
import { addIdeaNoteApi, updateIdeaNoteApi } from "@helpers/api/idea";
import { normalizeApiError } from "@helpers/helpers";

import Button from "@ds/Button";
import ModalTitleBar from "@components/global/ModalTitleBar";

interface NoteModalProps {
	ideaId: string;
	/** the note being edited; absent = adding a new one */
	note?: { id: string; text: string };
	onClose: () => void;
	/** called once the backend confirmed the change */
	onSaved: () => void;
}

/**
 * Add or edit a note. Notes are stored structurally on the idea and
 * rendered into its writeup ("## Notes") by the backend, so the Summary
 * tab and every export include them.
 */
export default function NoteModal({ ideaId, note, onClose, onSaved }: NoteModalProps) {
	const isEdit = note !== undefined;
	const [text, setText] = useState(note?.text ?? "");
	const [isSaving, setIsSaving] = useState(false);
	const [saveError, setSaveError] = useState<string | null>(null);
	const dialogRef = useRef<HTMLDialogElement | null>(null);
	const textareaRef = useRef<HTMLTextAreaElement | null>(null);

	// a save settling after the modal closed must not touch state
	const isMountedRef = useRef(true);
	useEffect(() => {
		isMountedRef.current = true;
		return () => {
			isMountedRef.current = false;
		};
	}, []);

	// belt and braces beyond the disabled button: a second activation
	// while a save is pending must not submit again
	const isSavingRef = useRef(false);
	const onSubmit = () => {
		const trimmed = text.trim();
		if (isSavingRef.current || !trimmed) return;
		isSavingRef.current = true;
		setSaveError(null);
		setIsSaving(true);
		const request = isEdit
			? updateIdeaNoteApi(ideaId, note.id, trimmed)
			: addIdeaNoteApi(ideaId, trimmed);
		request
			.then(() => {
				if (!isMountedRef.current) return;
				onSaved();
				onClose();
			})
			.catch((err) => {
				if (!isMountedRef.current) return;
				// keep the entered text so the user can simply retry
				setSaveError(normalizeApiError(err));
			})
			.finally(() => {
				isSavingRef.current = false;
				if (isMountedRef.current) setIsSaving(false);
			});
	};

	// The native dialog's top layer owns the semantics showModal gives:
	// Escape (via cancel), focus trapping and aria-modal handling. This
	// effect opens it, lands focus in the textarea and returns focus to
	// the opener on close; every close path goes through the dialog's
	// own close machinery so `onClose` fires exactly once.
	useEffect(() => {
		const dialog = dialogRef.current;
		if (!dialog) return;
		const previouslyFocused = document.activeElement as HTMLElement | null;
		dialog.showModal();
		textareaRef.current?.focus();
		return () => {
			dialog.close();
			previouslyFocused?.focus();
		};
	}, []);

	const requestClose = () => dialogRef.current?.close();

	const title = isEdit ? "Edit note" : "Add a note";
	return (
		<dialog
			ref={dialogRef}
			aria-label={title}
			onClose={onClose}
			// Escape synthesizes cancel; closing through the dialog keeps
			// one close path
			onCancel={requestClose}
			className="bg-transparent p-0 max-w-none w-auto"
		>
			{/* the visual + click-dismiss backdrop, inside the dialog so
			    the top layer keeps Escape and focus; aria-hidden keeps the
			    decorative layer out of the a11y tree (same as before the
			    native-dialog move) */}
			<div aria-hidden="true" className="fixed inset-0 bg-black/40" onClick={requestClose} />
			<div className="relative flex flex-col max-w-[600px] w-[90vw] bg-white mx-auto rounded-lg shadow-md p-5 md:p-7">
				<>
					<ModalTitleBar title={title} onClose={requestClose} classes="mb-2" />
					<p className="text-sm text-stone-500 mb-3">
						{isEdit
							? "Editing updates the note in the summary and in exports."
							: "The note joins this idea\u2019s summary under a Notes heading — exports include it too."}
					</p>
					<textarea
						ref={textareaRef}
						aria-label="Note"
						rows={5}
						value={text}
						onChange={(e) => setText(e.target.value)}
						placeholder="Anything you want to remember about this idea..."
						className="w-full text-sm px-4 py-2 border border-stone-200 rounded-md focus:outline-none focus:border-accent-500 resize-none md:resize-y"
					/>
					{saveError !== null && (
						<div
							role="alert"
							className="border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-3 text-sm mt-2"
						>
							<b>
								{isEdit
									? "Couldn\u2019t save the changes."
									: "Couldn\u2019t save the note."}
							</b>{" "}
							{saveError} Your text is still here — press Save to try again.
						</div>
					)}
					<div className="flex justify-end gap-3 mt-4">
						<Button variant="bordered" onClick={requestClose} disabled={isSaving}>
							Cancel
						</Button>
						<Button
							variant="pink"
							onClick={onSubmit}
							disabled={isSaving || text.trim() === ""}
						>
							{isSaving ? "Saving" : isEdit ? "Save changes" : "Save note"}
						</Button>
					</div>
				</>
			</div>
		</dialog>
	);
}
