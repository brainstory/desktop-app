import { atom } from "nanostores";
import { listen } from "@tauri-apps/api/event";
import { EVENTS } from "@src/tauri/commands";

/**
 * Live download state, shared across pages: the AI models rows, the
 * sidebar progress pill, and the chat's setup notice all read it. The
 * backend's model-download events (which carry the model label) are its
 * only source, so progress survives navigation - the settings page
 * neither has to have started the download nor be open. There is no
 * initial fetch: a page opened mid-download fills itself in on the next
 * progress chunk (events keep arriving during a transfer).
 */
export interface ActiveDownload {
	/** 0-100; negative when the backend cannot tell the total size */
	pct: number;
	label: string;
	/** bytes per second, when the backend could compute it */
	bytesPerSec?: number;
	/** estimated seconds left, when known */
	etaSecs?: number;
}

export interface DownloadFailure {
	label: string;
	/** plain-language description of what went wrong */
	message: string;
}

export interface DownloadsState {
	active: Record<string, ActiveDownload>;
	/** user-paused downloads: the transfer stopped keeping its `.part`
	 * (Resume continues from there), so these are not failures */
	paused: Record<string, ActiveDownload>;
	/** last failure per model id; a user cancel is never recorded here */
	failures: Record<string, DownloadFailure>;
}

const IDLE: DownloadsState = { active: {}, paused: {}, failures: {} };
export const $downloads = atom<DownloadsState>(IDLE);

export interface ModelDownloadEvent {
	modelId?: string;
	label?: string;
	kind?: string;
	pct?: number;
	bytesPerSec?: number;
	etaSecs?: number;
	message?: string;
}

/**
 * Turn the downloader's technical error strings into what a person can
 * act on. An unknown message falls through with its original text: a
 * confidently wrong simplification would be worse than the truth.
 */
export function formatDownloadError(raw: string): string {
	const message = raw.trim();
	if (message.startsWith("download failed with status")) {
		const status = message.replace("download failed with status", "").trim();
		return `The download server refused the request (${status}). Try again later.`;
	}
	const rules: { prefix: string; text: string }[] = [
		{
			prefix: "download stalled",
			text: "The download stalled — no data arrived for a while. Try again; it resumes where it left off."
		},
		{
			prefix: "download timed out waiting",
			text: "The download server didn't respond in time. Try again; it resumes where it left off."
		},
		{
			prefix: "download interrupted",
			text: "The connection dropped mid-download. Try again; it resumes where it left off."
		},
		{
			prefix: "download incomplete",
			text: "The download stopped early. Try again; it resumes where it left off."
		},
		{
			prefix: "download size mismatch",
			text: "The server is serving a different file than expected. Try again later; if it keeps failing, check the download endpoint under Model downloads."
		},
		{
			prefix: "download failed its integrity check",
			text: "The downloaded file didn't match its checksum, so it was discarded. Try again; if it keeps failing, check the download endpoint under Model downloads."
		},
		{
			prefix: "download request failed",
			text: "Couldn't reach the download server. Check your connection and try again."
		},
		{
			prefix: "download cancelled",
			text: "Download cancelled — nothing was kept."
		}
	];
	for (const rule of rules) {
		if (message.startsWith(rule.prefix)) {
			return rule.text;
		}
	}
	return `Download failed: ${message}`;
}

function applyEvent(event: ModelDownloadEvent): void {
	const { modelId, kind, label, pct, bytesPerSec, etaSecs, message } = event;
	if (!modelId || !kind) return;
	const state = $downloads.get();
	const active = { ...state.active };
	const paused = { ...state.paused };
	const failures = { ...state.failures };
	switch (kind) {
		case "progress":
			active[modelId] = {
				pct: typeof pct === "number" ? pct : -1,
				label: label ?? active[modelId]?.label ?? modelId,
				...(typeof bytesPerSec === "number" ? { bytesPerSec } : {}),
				...(typeof etaSecs === "number" ? { etaSecs } : {})
			};
			// an actively transferring model is no longer paused
			delete paused[modelId];
			delete failures[modelId];
			break;
		case "paused":
			delete active[modelId];
			delete failures[modelId];
			paused[modelId] = {
				pct: typeof pct === "number" ? pct : -1,
				label: label ?? paused[modelId]?.label ?? modelId
			};
			break;
		case "done":
		case "cancelled":
			delete active[modelId];
			delete paused[modelId];
			delete failures[modelId];
			break;
		case "error":
			delete active[modelId];
			delete paused[modelId];
			failures[modelId] = {
				label: label ?? failures[modelId]?.label ?? modelId,
				message: formatDownloadError(message ?? "")
			};
			break;
		// "load-error" (the engine failed to start) and "activated" (the
		// selection moved) change no transfer state here: the engine
		// status store and the models refresh own those.
		default:
			return;
	}
	$downloads.set({ active, paused, failures });
}

/** Drop a recorded failure, e.g. when its row is retried. */
export function clearDownloadFailure(modelId: string): void {
	const state = $downloads.get();
	if (!(modelId in state.failures)) return;
	const failures = { ...state.failures };
	delete failures[modelId];
	$downloads.set({ ...state, failures });
}

/**
 * Rehydrate in-flight (and paused) downloads from a models listing (the
 * settings page re-reads the catalog on mount): add the models it reports
 * unless the store already tracks them - an event pct is always fresher
 * than the listing's snapshot.
 */
export function seedDownloads(
	entries: Record<string, ActiveDownload>,
	pausedEntries: Record<string, ActiveDownload> = {}
): void {
	const state = $downloads.get();
	const active = { ...state.active };
	const paused = { ...state.paused };
	let changed = false;
	for (const [id, entry] of Object.entries(entries)) {
		if (!(id in active)) {
			active[id] = entry;
			changed = true;
		}
	}
	for (const [id, entry] of Object.entries(pausedEntries)) {
		if (!(id in paused) && !(id in active)) {
			paused[id] = entry;
			changed = true;
		}
	}
	if (changed) {
		$downloads.set({ ...state, active, paused });
	}
}

/**
 * Keep the download feed live while at least one component is subscribed
 * to it (the sidebar pill on every page, the models card in settings).
 * Refcounted on purpose: the listener exists exactly while the app shows
 * something that needs it, and a React effect can own it directly - no
 * store-lifecycle indirection.
 */
let liveCount = 0;
let stopListening: (() => void) | null = null;

export function subscribeDownloads(): () => void {
	liveCount++;
	if (liveCount === 1) {
		void listen<ModelDownloadEvent>(EVENTS.modelDownload, (event) =>
			applyEvent(event.payload ?? {})
		).then((unlisten) => {
			// a subscriber that left before the registration resolved
			// releases the listener itself; otherwise it is the shared one
			if (liveCount > 0 && stopListening === null) {
				stopListening = unlisten;
			} else {
				unlisten();
			}
		});
	}
	return () => {
		liveCount = Math.max(0, liveCount - 1);
		if (liveCount === 0) {
			stopListening?.();
			stopListening = null;
			$downloads.set(IDLE);
		}
	};
}
