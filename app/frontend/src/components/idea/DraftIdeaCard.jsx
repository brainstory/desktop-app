import { useState } from "react";
import { formatISO8601ToHumanReadable } from "../../helpers/helpers";
import { deleteIdeaApi } from "@helpers/api/idea";

export default function DraftIdeaCard({ id, createdAt, draftSummary }) {
	const humanReadableDate = formatISO8601ToHumanReadable(createdAt);
	const [confirmingDelete, setConfirmingDelete] = useState(false);

	const handleDelete = (e) => {
		e.preventDefault();
		e.stopPropagation();
		if (!confirmingDelete) {
			setConfirmingDelete(true);
			setTimeout(() => setConfirmingDelete(false), 5000);
			return;
		}
		deleteIdeaApi(id)
			.then(() => window.location.reload())
			.catch((err) => console.log("delete failed", err));
	};

	return (
		<div className="relative h-auto w-80 sm:w-[275px] max-w-sm p-6 bg-white border-2 border-pink-200 rounded-lg shadow hover:shadow-lg hover:-translate-y-1 transition-transform">
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

				<p className="mb-3 text-xs text-stone-400">{humanReadableDate}</p>
				<div className="inline-flex items-center justify-center w-full">
					<hr className="w-64 h-[2px] my-8 bg-stone-200 border-0 rounded" />
					<div className="absolute px-4 -translate-x-1/2 bg-white left-1/2 uppercase font-bold text-stone-400 text-xs">
						last message
					</div>
				</div>
				<p className="mb-1 text-stone-400 italic break-words line-clamp-5 text-sm">
					{draftSummary}
				</p>
			</a>
			<button
				onClick={handleDelete}
				aria-label="Delete draft"
				className={`absolute top-2 right-2 p-1.5 rounded-full text-xs font-semibold ${
					confirmingDelete
						? "text-red-600 bg-red-50"
						: "text-stone-300 hover:text-red-500 hover:bg-stone-50"
				}`}
			>
				{confirmingDelete ? (
					"Really delete?"
				) : (
					<ion-icon class="w-4 h-4 hydrated" name="trash-outline" role="img"></ion-icon>
				)}
			</button>
		</div>
	);
}
