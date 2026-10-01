import { formatISO8601ToHumanReadable } from "../../helpers/helpers";
import { deleteIdeaApi } from "@helpers/api/idea";
import { CONFIRM_DELETE_ANNOUNCEMENT, useConfirmClick } from "@src/hooks/useTimeout";
import { Snackbar } from "@ds/Snackbar";
import { useState } from "react";

interface DraftIdeaCardProps {
	id: string;
	createdAt?: string | null;
	draftSummary?: string | null;
	/** Called after a successful delete so the parent updates its list
	 * (no page reload). */
	onDeleted?: (id: string) => void;
}

export default function DraftIdeaCard({
	id,
	createdAt,
	draftSummary,
	onDeleted
}: DraftIdeaCardProps) {
	const humanReadableDate = formatISO8601ToHumanReadable(createdAt ?? "");
	const { isConfirming: confirmingDelete, secondsLeft, confirm } = useConfirmClick();
	const [deleteFailed, setDeleteFailed] = useState(false);

	const handleDelete = (): void => {
		if (!confirm()) return;
		deleteIdeaApi(id)
			.then(() => onDeleted?.(id))
			.catch((err) => {
				console.error("delete failed", err);
				setDeleteFailed(true);
			});
	};

	return (
		<div className="relative h-auto w-80 sm:w-[275px] max-w-sm p-6 bg-white border-2 border-pink-200 rounded-lg shadow hover:shadow-lg hover:-translate-y-1 transition-transform">
			{deleteFailed && (
				<Snackbar
					isSuccess={false}
					message="Error: Could not delete this draft"
					onClose={() => setDeleteFailed(false)}
				/>
			)}
			<a href={`/chat?id=${id}`} className="block">
				<div className="float-left">
					<span className="flex items-center justify-center mt-1 mr-2 w-6 h-6 bg-pink-100 text-pink-600 rounded-full -left-4 ring-8 ring-white">
						<ion-icon
							class="md hydrated"
							name="ellipsis-horizontal"
							role="img"
							aria-label="draft"
						></ion-icon>
					</span>
				</div>
				<h3 className="mb-2 text-xl font-semibold tracking-tight text-stone-900">Draft</h3>

				<p className="mb-3 text-xs text-stone-500">{humanReadableDate}</p>
				<div className="inline-flex items-center justify-center w-full">
					<hr className="w-64 h-[2px] my-8 bg-stone-200 border-0 rounded" />
					<div className="absolute px-4 -translate-x-1/2 bg-white left-1/2 uppercase font-bold text-stone-500 text-xs">
						last message
					</div>
				</div>
				<p className="mb-1 text-stone-400 italic break-words line-clamp-5 text-sm">
					{draftSummary}
				</p>
			</a>
			<button
				type="button"
				onClick={handleDelete}
				aria-label={confirmingDelete ? "Confirm delete draft" : "Delete draft"}
				className={`absolute top-2 right-2 p-1.5 rounded-full text-xs font-semibold ${
					confirmingDelete
						? "text-red-600 bg-red-50"
						: "text-stone-500 hover:text-red-500 hover:bg-stone-50"
				}`}
			>
				{confirmingDelete ? (
					<span>Really delete? ({secondsLeft ?? 0}s)</span>
				) : (
					<ion-icon class="w-4 h-4 hydrated" name="trash-outline" role="img"></ion-icon>
				)}
			</button>
			{/* Announced once when armed - outside the button (its aria-label
				would override it) and not re-announced on every tick */}
			<span role="status" className="sr-only">
				{confirmingDelete ? CONFIRM_DELETE_ANNOUNCEMENT : ""}
			</span>
		</div>
	);
}
