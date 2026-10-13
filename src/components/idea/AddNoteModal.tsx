import { useEffect, useRef, useState } from "react";
import { addIdeaNoteApi } from "@helpers/api/idea";
import { normalizeApiError } from "@helpers/helpers";

import Button from "@ds/Button";
import ModalTitleBar from "@components/global/ModalTitleBar";

interface AddNoteModalProps {
	ideaId: string;
	onClose: () => void;
	/** called once the backend confirmed the note */
	onSaved: () => void;
}

/**
 * A quick text note appended to an idea's writeup ("## Notes"). Notes
 * live in the same markdown result as the writeup, so the Summary tab
 * and every export include them - no separate storage to keep in sync.
 */
export default function AddNoteModal({ ideaId, onClose, onSaved }: AddNoteModalProps) {
	const [note, setNote] = useState("");
	const [isSaving, setIsSaving] = useState(false);
	const [saveError, setSaveError] = useState<string | null>(null);
	const modalRef = useRef<HTMLDivElement | null>(null);
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
		const trimmed = note.trim();
		if (isSavingRef.current || !trimmed) return;
		isSavingRef.current = true;
		setSaveError(null);
		setIsSaving(true);
		addIdeaNoteApi(ideaId, trimmed)
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

	// Escape closes; Tab is trapped; focus starts in the textarea and
	// returns to the opener on close.
	const onCloseRef = useRef(onClose);
	useEffect(() => {
		onCloseRef.current = onClose;
	}, [onClose]);

	useEffect(() => {
		const modal = modalRef.current;
		const previouslyFocused = document.activeElement as HTMLElement | null;
		const focusableSelector =
			'a[href], button:not([disabled]), textarea, input, select, [tabindex]:not([tabindex="-1"])';
		const focusables = (): HTMLElement[] =>
			Array.from(modal?.querySelectorAll<HTMLElement>(focusableSelector) ?? []);

		textareaRef.current?.focus();

		const onKeyDown = (event: globalThis.KeyboardEvent): void => {
			if (event.key === "Escape") {
				onCloseRef.current();
				return;
			}
			if (event.key === "Tab") {
				const items = focusables();
				if (items.length === 0) return;
				const first = items[0]!;
				const last = items[items.length - 1]!;
				if (event.shiftKey && document.activeElement === first) {
					event.preventDefault();
					last.focus();
				} else if (!event.shiftKey && document.activeElement === last) {
					event.preventDefault();
					first.focus();
				}
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => {
			window.removeEventListener("keydown", onKeyDown);
			previouslyFocused?.focus();
		};
	}, []);

	return (
		<div
			role="dialog"
			aria-modal="true"
			aria-label="Add a note"
			className="fixed inset-0 z-50 flex items-center justify-center"
		>
			{/* backdrop: click to dismiss */}
			<div
				className="fixed inset-0 bg-black opacity-40"
				aria-hidden="true"
				onClick={onClose}
			></div>
			<div
				ref={modalRef}
				className="flex flex-col max-w-[600px] w-[90vw] bg-white mx-auto rounded-lg relative shadow-md p-5 md:p-7"
			>
				<ModalTitleBar title="Add a note" onClose={onClose} classes="mb-2" />
				<p className="text-sm text-stone-500 mb-3">
					The note joins this idea&rsquo;s summary under a Notes heading — exports include
					it too.
				</p>
				<textarea
					ref={textareaRef}
					aria-label="Note"
					rows={5}
					value={note}
					onChange={(e) => setNote(e.target.value)}
					placeholder="Anything you want to remember about this idea..."
					className="w-full text-sm px-4 py-2 border border-stone-200 rounded-md focus:outline-none focus:border-accent-500 resize-none md:resize-y"
				/>
				{saveError !== null && (
					<div
						role="alert"
						className="border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-3 text-sm mt-2"
					>
						<b>Couldn&rsquo;t save the note.</b> {saveError} Your text is still here —
						press Save to try again.
					</div>
				)}
				<div className="flex justify-end gap-3 mt-4">
					<Button variant="bordered" onClick={onClose} disabled={isSaving}>
						Cancel
					</Button>
					<Button
						variant="pink"
						onClick={onSubmit}
						disabled={isSaving || note.trim() === ""}
					>
						{isSaving ? "Saving" : "Save note"}
					</Button>
				</div>
			</div>
		</div>
	);
}
