import { useState } from "react";
import { useStore } from "@nanostores/react";

import type { IdeaNote } from "@src/types";
import { deleteIdeaNoteApi } from "@helpers/api/idea";
import { formatISO8601ToHumanReadable, normalizeApiError } from "@helpers/helpers";
import { useConfirmClick } from "@src/hooks/useTimeout";
import { $userState } from "@components/global/userStore";
import NoteModal from "./NoteModal";

interface IdeaNotesProps {
	ideaId: string;
	notes: IdeaNote[];
	/** refresh the idea after a note changed */
	onChanged: () => void;
}

/**
 * The user's notes on an idea, shown with the summary: one row per note,
 * each editable and removable. The backend rebuilds the writeup's
 * "## Notes" section from these on every change, so the document and
 * exports stay in sync.
 */
export default function IdeaNotes({ ideaId, notes, onChanged }: IdeaNotesProps) {
	const [editing, setEditing] = useState<IdeaNote | null>(null);
	if (notes.length === 0) {
		return null;
	}
	return (
		<div className="mt-6 border-t border-stone-200 pt-4 text-sm">
			<h3 className="font-semibold tracking-wide text-stone-700 uppercase text-xs mb-3">
				Your notes
			</h3>
			<ul className="flex flex-col gap-4">
				{notes.map((note) => (
					<NoteRow
						key={note.id}
						ideaId={ideaId}
						note={note}
						onEdit={() => setEditing(note)}
						onRemoved={onChanged}
					/>
				))}
			</ul>
			{editing && (
				<NoteModal
					ideaId={ideaId}
					note={{ id: editing.id, text: editing.text }}
					onClose={() => setEditing(null)}
					onSaved={onChanged}
				/>
			)}
		</div>
	);
}

function NoteRow({
	ideaId,
	note,
	onEdit,
	onRemoved
}: {
	ideaId: string;
	note: IdeaNote;
	onEdit: () => void;
	onRemoved: () => void;
}) {
	const { timezone } = useStore($userState);
	const { isConfirming, secondsLeft, confirm } = useConfirmClick();
	const [error, setError] = useState<string | null>(null);
	// a delete that settled after a re-render must not fire twice
	const [removing, setRemoving] = useState(false);

	const remove = () => {
		if (removing) return;
		setRemoving(true);
		setError(null);
		deleteIdeaNoteApi(ideaId, note.id)
			.then(onRemoved)
			.catch((err) => {
				setError(normalizeApiError(err));
				setRemoving(false);
			});
	};

	return (
		<li className="flex items-start gap-3">
			<div className="flex-1 min-w-0">
				<p className="whitespace-pre-wrap break-words text-stone-800">{note.text}</p>
				<p className="text-xs text-stone-500 mt-1">
					{formatISO8601ToHumanReadable(
						note.createdAt,
						{ year: "numeric", month: "short", day: "numeric" },
						timezone ?? undefined
					)}
				</p>
				{error !== null && (
					<p role="alert" className="text-xs text-red-600 mt-1">
						{error}
					</p>
				)}
			</div>
			<div className="flex gap-2 shrink-0">
				<button
					type="button"
					className="underline text-stone-600 hover:no-underline"
					onClick={onEdit}
				>
					Edit
				</button>
				<button
					type="button"
					className={`underline hover:no-underline ${isConfirming ? "text-red-600 font-semibold" : "text-stone-600"}`}
					onClick={() => {
						if (confirm()) remove();
					}}
				>
					{isConfirming ? `Remove? (${secondsLeft ?? 0}s)` : "Remove"}
				</button>
			</div>
			{isConfirming && (
				<span role="status" className="sr-only">
					{`Click again to remove the note, resets in ${secondsLeft ?? 0} seconds`}
				</span>
			)}
		</li>
	);
}
