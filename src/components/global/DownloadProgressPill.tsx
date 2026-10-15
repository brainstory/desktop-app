import { useStore } from "@nanostores/react";
import { useEffect } from "react";
import { $downloads, clearDownloadFailure, subscribeDownloads } from "./downloadStore";
import Icon from "@ds/Icon";

/**
 * Sidebar shortcut to whatever download is in flight (or failed), so
 * progress is visible from every page, not only the settings page that
 * may have started it. Renders nothing when there is nothing to say -
 * but keeps the shared download feed live on pages that have no AI
 * models card.
 */
export default function DownloadProgressPill() {
	// the pill is on every page: it is the feed's dependable subscriber
	useEffect(() => subscribeDownloads(), []);
	const { active, paused, failures } = useStore($downloads);
	// a paused download stays visible: Resume must be reachable from
	// every page, not only the settings one
	const downloads = [...Object.entries(active), ...Object.entries(paused)];
	const pausedIds = new Set(Object.keys(paused));
	const failed = Object.entries(failures);
	if (downloads.length === 0 && failed.length === 0) return null;

	let icon = "cloud-download-outline";
	let title: string;
	let detail = "";
	let isError = false;
	if (downloads.length === 1) {
		const [id, first] = downloads[0]!;
		title = first.label;
		if (pausedIds.has(id)) {
			detail =
				first.pct >= 0 ? `Paused · ${Math.min(100, Math.floor(first.pct))}%` : "Paused";
			icon = "pause-circle-outline";
		} else {
			detail = first.pct < 0 ? "downloading…" : `${Math.min(100, Math.floor(first.pct))}%`;
		}
	} else if (downloads.length > 1) {
		title = `${downloads.length} models downloading`;
	} else {
		// only failures remain
		icon = "alert-circle-outline";
		title = failed.length === 1 ? failed[0]![1].label : `${failed.length} downloads failed`;
		detail = "Download failed — review";
		isError = true;
	}
	const singleFailureId = failed.length === 1 ? failed[0]![0] : null;
	const pct =
		downloads.length === 1 && downloads[0]![1].pct >= 0
			? Math.min(100, Math.floor(downloads[0]![1].pct))
			: null;

	return (
		<a
			href="/profile?tab=aiModels"
			title="Open AI Models"
			className={`mx-5 mt-4 flex items-center gap-2 rounded-lg border px-3 py-2 ${
				isError
					? "border-red-200 bg-red-50 text-red-800 hover:bg-red-100"
					: "border-stone-200 bg-white text-stone-700 hover:bg-stone-100"
			}`}
		>
			<Icon name={icon} className="h-5 w-5 shrink-0" ariaHidden />
			<span className="min-w-0 flex-1 text-left">
				<span className="block truncate text-xs font-medium">{title}</span>
				{detail && <span className="block text-[11px] text-stone-500">{detail}</span>}
				{pct !== null && (
					<span className="mt-1 block h-1 w-full overflow-hidden rounded-full bg-stone-200">
						<span
							className="block h-1 rounded-full bg-pink-500 transition-all"
							style={{ width: `${pct}%` }}
						/>
					</span>
				)}
			</span>
			{singleFailureId !== null && (
				<button
					type="button"
					aria-label="Dismiss download failure"
					className="shrink-0 rounded-full px-1.5 text-lg leading-none text-red-400 hover:text-red-600"
					onClick={(event) => {
						event.preventDefault();
						event.stopPropagation();
						clearDownloadFailure(singleFailureId);
					}}
				>
					×
				</button>
			)}
		</a>
	);
}
