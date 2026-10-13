import { useStore } from "@nanostores/react";
import { useEffect } from "react";
import { $downloads, subscribeDownloads } from "./downloadStore";

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
	const { active, failures } = useStore($downloads);
	const downloads = Object.entries(active);
	const failed = Object.entries(failures);
	if (downloads.length === 0 && failed.length === 0) return null;

	let icon = "cloud-download-outline";
	let title: string;
	let detail = "";
	let isError = false;
	if (downloads.length === 1) {
		const [, first] = downloads[0]!;
		title = first.label;
		detail = first.pct < 0 ? "downloading…" : `${Math.min(100, Math.floor(first.pct))}%`;
	} else if (downloads.length > 1) {
		title = `${downloads.length} models downloading`;
	} else {
		// only failures remain
		icon = "alert-circle-outline";
		title = failed.length === 1 ? failed[0]![1].label : `${failed.length} downloads failed`;
		detail = "Download failed — review";
		isError = true;
	}
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
			<ion-icon name={icon} class="hydrated h-5 w-5 shrink-0" aria-hidden="true" />
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
		</a>
	);
}
