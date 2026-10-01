import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getUpdatesEnabledApi } from "@helpers/api/settings";
import { STORAGE_KEYS } from "@src/tauri/commands";

/**
 * Update check + install banner. Only active inside the Tauri webview
 * (the astro build also runs as plain web pages, where the plugin APIs
 * don't exist). Checks are throttled to once per 6 hours via localStorage
 * to stay well clear of anonymous GitHub rate limits, and skipped
 * entirely when the user turned automatic checks off in Settings.
 */

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const THROTTLE_KEY = STORAGE_KEYS.lastUpdateCheckMs;
const THROTTLE_MS = 6 * 60 * 60 * 1000;

type Phase = "available" | "downloading" | "restarting" | "error";

export default function UpdaterBanner() {
	const [update, setUpdate] = useState<Update | null>(null);
	const [phase, setPhase] = useState<Phase | null>(null);
	const [downloaded, setDownloaded] = useState(0);
	const [total, setTotal] = useState<number | undefined>(undefined);

	useEffect(() => {
		if (!isTauri) return;
		let last = 0;
		try {
			last = Number(localStorage.getItem(THROTTLE_KEY) ?? 0);
		} catch {
			// storage unavailable: fall through and check unthrottled
		}
		if (Date.now() - last < THROTTLE_MS) return;
		const run = async () => {
			let enabled = true;
			try {
				enabled = await getUpdatesEnabledApi();
			} catch (err) {
				// the backend defaults to on; an unreadable setting keeps that default
				console.error("reading the update setting failed", err);
			}
			// opted out in Settings > Updates: never contact the update server
			if (!enabled) return;
			const u = await check();
			// Throttle only after a check completed: writing the key
			// before check() meant an offline failure suppressed update
			// checks for the next 6 hours.
			try {
				localStorage.setItem(THROTTLE_KEY, String(Date.now()));
			} catch {
				// ignore
			}
			if (u) {
				setUpdate(u);
				// entering the "available" phase is what makes the banner
				// render at all - without this the update is never shown
				setPhase("available");
			}
		};
		run().catch((err) => console.error("update check failed", err));
	}, []);

	if (!isTauri || !update || phase === null) return null;

	const install = async () => {
		try {
			setPhase("downloading");
			await update.downloadAndInstall((event) => {
				if (event.event === "Started") setTotal(event.data.contentLength ?? undefined);
				if (event.event === "Progress") setDownloaded((n) => n + event.data.chunkLength);
			});
			setPhase("restarting");
			await relaunch();
		} catch (err) {
			console.error("update install failed", err);
			setPhase("error");
		}
	};

	const pct = total ? Math.round((downloaded / total) * 100) : undefined;

	return (
		<div
			role="status"
			className="fixed bottom-4 left-1/2 -translate-x-1/2 z-50 bg-stone-900 text-white rounded-lg shadow-xl px-5 py-3 flex items-center gap-4 text-sm w-[min(92vw,480px)]"
		>
			<div className="flex-1 min-w-0">
				<p className="font-semibold">Update available: {update.version}</p>
				{phase === "downloading" && (
					<div className="mt-1.5">
						<div className="w-full h-1.5 bg-stone-700 rounded-full overflow-hidden">
							<div
								className="h-full bg-pink-500 rounded-full transition-all"
								style={{ width: pct === undefined ? "40%" : `${pct}%` }}
							/>
						</div>
						<p className="text-xs text-stone-400 mt-1">
							{pct === undefined ? "Downloading…" : `Downloading… ${pct}%`}
						</p>
					</div>
				)}
				{phase === "restarting" && <p className="text-xs text-stone-400">Restarting…</p>}
				{phase === "error" && (
					<p className="text-xs text-red-300">Update failed — try again later.</p>
				)}
			</div>
			{phase === "available" && (
				<button
					className="bg-accent-600 hover:bg-accent-700 text-white font-medium rounded-md px-3 py-1.5 shrink-0"
					onClick={install}
				>
					Restart to update
				</button>
			)}
		</div>
	);
}
