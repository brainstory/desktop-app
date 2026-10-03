import { useEffect, useRef, useState } from "react";
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
 *
 * Astro pages are separate documents: navigating unmounts this component
 * and destroys the native Update resource with them. Only safe metadata
 * (version + check time) is persisted; a fresh mount restores the banner
 * from it without any network call, and the install button reacquires a
 * live Update with one user-initiated check before installing.
 */

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const THROTTLE_KEY = STORAGE_KEYS.lastUpdateCheckMs;
const THROTTLE_MS = 6 * 60 * 60 * 1000;
/** metadata about a known-available update; never persists native resource ids */
const AVAILABLE_UPDATE_KEY = "updater_available_update";

type PersistedUpdate = { version: string; checkedAtMs: number };

type Phase = "available" | "downloading" | "restarting" | "installedPending" | "error";

const readPersistedUpdate = (): PersistedUpdate | null => {
	try {
		const raw = localStorage.getItem(AVAILABLE_UPDATE_KEY);
		if (!raw) return null;
		const parsed = JSON.parse(raw) as Partial<PersistedUpdate>;
		if (typeof parsed.version !== "string" || parsed.version.length === 0) return null;
		if (typeof parsed.checkedAtMs !== "number" || !Number.isFinite(parsed.checkedAtMs))
			return null;
		return { version: parsed.version, checkedAtMs: parsed.checkedAtMs };
	} catch {
		return null;
	}
};

const writePersistedUpdate = (version: string) => {
	try {
		localStorage.setItem(
			AVAILABLE_UPDATE_KEY,
			JSON.stringify({ version, checkedAtMs: Date.now() } satisfies PersistedUpdate)
		);
	} catch {
		// ignore
	}
};

const clearPersistedUpdate = () => {
	try {
		localStorage.removeItem(AVAILABLE_UPDATE_KEY);
	} catch {
		// ignore
	}
};

const writeThrottle = () => {
	try {
		localStorage.setItem(THROTTLE_KEY, String(Date.now()));
	} catch {
		// ignore
	}
};

/** release a native update resource that nobody will install */
const closeDiscarded = (u: Update | null) => {
	if (!u) return;
	u.close().catch((err) => console.error("closing update resource failed", err));
};

export default function UpdaterBanner() {
	const [version, setVersion] = useState<string | null>(null);
	const [phase, setPhase] = useState<Phase | null>(null);
	const [downloaded, setDownloaded] = useState(0);
	const [total, setTotal] = useState<number | undefined>(undefined);
	// the live native Update (holds a backend resource); null while the
	// banner is only restored from persisted metadata
	const heldRef = useRef<Update | null>(null);
	const mountedRef = useRef(true);

	useEffect(() => {
		if (!isTauri) return;
		mountedRef.current = true;
		const restoreOrCheck = async () => {
			const persisted = readPersistedUpdate();
			// a check within the last 6 hours found an update: show the
			// banner again without any network call; the install button
			// reacquires a live Update on demand
			if (persisted && Date.now() - persisted.checkedAtMs < THROTTLE_MS) {
				setVersion(persisted.version);
				setPhase("available");
				return;
			}
			let last = 0;
			try {
				last = Number(localStorage.getItem(THROTTLE_KEY) ?? 0);
			} catch {
				// storage unavailable: fall through and check unthrottled
			}
			if (Date.now() - last < THROTTLE_MS) return;
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
			writeThrottle();
			if (!mountedRef.current) {
				// the banner unmounted while the check was in flight; nobody
				// will consume this resource, so release it
				closeDiscarded(u);
				return;
			}
			if (u) {
				heldRef.current = u;
				setVersion(u.version);
				// entering the "available" phase is what makes the banner
				// render at all - without this the update is never shown
				setPhase("available");
				writePersistedUpdate(u.version);
			} else {
				clearPersistedUpdate();
			}
		};
		restoreOrCheck().catch((err) => console.error("update check failed", err));
		return () => {
			mountedRef.current = false;
			// release a held-but-uninstalled update exactly once; a
			// successfully downloadAndInstall-ed one was already consumed
			// by the backend (nothing left to close, per the plugin typings)
			const held = heldRef.current;
			heldRef.current = null;
			closeDiscarded(held);
		};
	}, []);

	if (!isTauri || !version || phase === null) return null;

	const startInstall = async () => {
		try {
			let u = heldRef.current;
			if (!u) {
				// banner restored from persisted metadata: reacquire a live
				// Update with one user-initiated check before installing
				let enabled = true;
				try {
					enabled = await getUpdatesEnabledApi();
				} catch (err) {
					console.error("reading the update setting failed", err);
				}
				// opted out in Settings > Updates: never contact the update server
				if (!enabled) {
					setPhase("error");
					return;
				}
				u = await check();
				writeThrottle();
				if (!mountedRef.current) {
					closeDiscarded(u);
					return;
				}
				if (!u) {
					// the update was installed elsewhere in the meantime:
					// drop the stale metadata and the banner with it
					clearPersistedUpdate();
					setVersion(null);
					setPhase(null);
					return;
				}
				heldRef.current = u;
				setVersion(u.version);
				writePersistedUpdate(u.version);
			}
			if (!mountedRef.current) return;
			setPhase("downloading");
			// every attempt starts from zero bytes again
			setDownloaded(0);
			setTotal(undefined);
			await u.downloadAndInstall((event) => {
				if (event.event === "Started") setTotal(event.data.contentLength ?? undefined);
				if (event.event === "Progress") setDownloaded((n) => n + event.data.chunkLength);
			});
			// a successful downloadAndInstall consumed the backend resource
			heldRef.current = null;
			// the update is installed now; never advertise it again
			clearPersistedUpdate();
			if (!mountedRef.current) return;
			setPhase("restarting");
			try {
				await relaunch();
			} catch (err) {
				// install succeeded, only the restart failed: offer a plain
				// restart, never a second install
				console.error("relaunch after update install failed", err);
				if (mountedRef.current) setPhase("installedPending");
			}
		} catch (err) {
			console.error("update install failed", err);
			// the held update stays usable: the retry reuses it without
			// another network check
			if (mountedRef.current) setPhase("error");
		}
	};

	const retryRelaunch = async () => {
		try {
			await relaunch();
		} catch (err) {
			console.error("relaunch after update install failed", err);
		}
	};

	const pct = total ? Math.round((downloaded / total) * 100) : undefined;

	return (
		<div
			role="status"
			className="fixed bottom-4 left-1/2 -translate-x-1/2 z-50 bg-stone-900 text-white rounded-lg shadow-xl px-5 py-3 flex items-center gap-4 text-sm w-[min(92vw,480px)]"
		>
			<div className="flex-1 min-w-0">
				<p className="font-semibold">Update available: {version}</p>
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
				{phase === "installedPending" && (
					<p className="text-xs text-stone-300">Update installed — restart pending.</p>
				)}
				{phase === "error" && (
					<p className="text-xs text-red-300">Update failed — try again later.</p>
				)}
			</div>
			{phase === "available" && (
				<button
					className="bg-accent-600 hover:bg-accent-700 text-white font-medium rounded-md px-3 py-1.5 shrink-0"
					onClick={() => void startInstall()}
				>
					Restart to update
				</button>
			)}
			{phase === "error" && (
				<button
					className="bg-accent-600 hover:bg-accent-700 text-white font-medium rounded-md px-3 py-1.5 shrink-0"
					onClick={() => void startInstall()}
				>
					Try again
				</button>
			)}
			{phase === "installedPending" && (
				<button
					className="bg-accent-600 hover:bg-accent-700 text-white font-medium rounded-md px-3 py-1.5 shrink-0"
					onClick={() => void retryRelaunch()}
				>
					Restart now
				</button>
			)}
		</div>
	);
}
