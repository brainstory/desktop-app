import { useEffect, useState } from "react";
import Button from "@ds/Button";
import { useConfirmClick } from "@src/hooks/useTimeout";

interface LastAnswerActionsProps {
	/** the user's most recent answer */
	lastAnswer: string;
	/** replace the answer and ask the coach again from the corrected text */
	onEdit: (newText: string) => void;
	/** undo the answer (and the reply based on it) */
	onRemove: () => void;
}

/**
 * "That's not what I said" recovery for the most recent answer (voice
 * transcription included): shows what was captured, with Edit and
 * Remove. Editing re-asks the coach from the corrected text; removing
 * falls back to the previous question. The draft autosave follows both.
 */
export default function LastAnswerActions({
	lastAnswer,
	onEdit,
	onRemove
}: LastAnswerActionsProps) {
	const [isEditing, setIsEditing] = useState(false);
	const [draft, setDraft] = useState(lastAnswer);
	const { isConfirming, secondsLeft, confirm } = useConfirmClick();

	// Keyboard inside the edit box: Escape discards, Cmd/Ctrl+Enter
	// saves (the buttons remain the discoverable path). Re-attaches on
	// each keystroke so the handler always sees the current draft.
	useEffect(() => {
		if (!isEditing) return;
		const onKeyDown = (event: KeyboardEvent): void => {
			if (event.key === "Escape") {
				event.preventDefault();
				setIsEditing(false);
			} else if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
				event.preventDefault();
				const trimmed = draft.trim();
				setIsEditing(false);
				if (trimmed && trimmed !== lastAnswer) {
					onEdit(trimmed);
				}
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, [isEditing, draft, lastAnswer, onEdit]);

	const startEditing = (): void => {
		setDraft(lastAnswer);
		setIsEditing(true);
	};
	const saveEdit = (): void => {
		const trimmed = draft.trim();
		setIsEditing(false);
		if (!trimmed || trimmed === lastAnswer) {
			return;
		}
		onEdit(trimmed);
	};

	return (
		<div className="mx-auto w-full max-w-2xl text-sm text-stone-500 mb-2">
			{isEditing ? (
				<div className="flex flex-col gap-2">
					<textarea
						aria-label="Edit your last answer"
						rows={3}
						value={draft}
						onChange={(e) => setDraft(e.target.value)}
						className="w-full text-sm px-4 py-2 border border-stone-200 rounded-md focus:outline-none focus:border-accent-500 resize-none bg-white text-stone-900"
					/>
					<div className="flex gap-2 justify-end">
						<Button variant="transparent" onClick={() => setIsEditing(false)}>
							Cancel
						</Button>
						<Button variant="pink" disabled={draft.trim() === ""} onClick={saveEdit}>
							Save &amp; ask again
						</Button>
					</div>
				</div>
			) : (
				<div className="flex items-start gap-3">
					<p className="flex-1 min-w-0 line-clamp-2">
						<span className="font-medium text-stone-600">Your last answer:</span>{" "}
						<span className="italic">&ldquo;{lastAnswer}&rdquo;</span>
					</p>
					<button
						className="underline hover:no-underline whitespace-nowrap"
						onClick={startEditing}
					>
						Edit
					</button>
					<button
						className={`underline hover:no-underline whitespace-nowrap ${isConfirming ? "text-red-600 font-semibold" : ""}`}
						onClick={() => {
							if (confirm()) onRemove();
						}}
					>
						{isConfirming ? `Remove? (${secondsLeft ?? 0}s)` : "Remove"}
					</button>
				</div>
			)}
			{isConfirming && (
				<span role="status" className="sr-only">
					{`Click again to remove the answer, resets in ${secondsLeft ?? 0} seconds`}
				</span>
			)}
		</div>
	);
}
