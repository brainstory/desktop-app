import { atom, onMount } from "nanostores";
import type { DownloadEvent } from "@src/tauri/generated/DownloadEvent";
import { listenEvent } from "@src/tauri/events";

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

function applyEvent(event: DownloadEvent): void {
	const { kind } = event;
	const modelId = event.modelId;
	const label = event.label;
	// channel-shaped events (no model id) carry no state for the shared
	// feed: the global event with the same kind follows immediately
	if (!modelId) return;
	const state = $downloads.get();
	const active = { ...state.active };
	const paused = { ...state.paused };
	const failures = { ...state.failures };
	switch (kind) {
		case "progress":
			active[modelId] = {
				pct: event.pct,
				// after a resume, the paused entry still holds the label
				label: label ?? active[modelId]?.label ?? paused[modelId]?.label ?? modelId,
				...(event.bytesPerSec !== undefined ? { bytesPerSec: event.bytesPerSec } : {}),
				...(event.etaSecs !== undefined ? { etaSecs: event.etaSecs } : {})
			};
			// an actively transferring model is no longer paused
			delete paused[modelId];
			delete failures[modelId];
			break;
		case "paused":
			delete failures[modelId];
			paused[modelId] = {
				pct: event.pct,
				// keep the label the transfer was showing before it paused
				label: label ?? paused[modelId]?.label ?? active[modelId]?.label ?? modelId
			};
			delete active[modelId];
			break;
		case "done":
		case "cancelled":
			delete active[modelId];
			delete paused[modelId];
			delete failures[modelId];
			break;
		case "load-error":
			failures[modelId] = {
				// keep the label the failed transfer was showing
				label:
					label ??
					failures[modelId]?.label ??
					active[modelId]?.label ??
					paused[modelId]?.label ??
					modelId,
				message: formatDownloadError(event.message)
			};
			delete active[modelId];
			delete paused[modelId];
			break;
		case "error":
			failures[modelId] = {
				// keep the label the failed transfer was showing
				label:
					label ??
					failures[modelId]?.label ??
					active[modelId]?.label ??
					paused[modelId]?.label ??
					modelId,
				message: formatDownloadError(event.message)
			};
			delete active[modelId];
			delete paused[modelId];
			break;
		// "activated" (the selection moved) changes no transfer state here:
		// the engine status store and the models refresh own those.
		default:
			return;
	}
	$downloads.set({ active, paused, failures });
}

/** Drop a recorded failure, e.g. when its row is retried. */
/** Optimistically seed an in-flight download so the UI reacts the
 * moment the button is pressed (backend progress events follow only
 * once the transfer starts producing them). */
export function markDownloadStarting(modelId: string, label: string): void {
	const state = $downloads.get();
	$downloads.set({
		...state,
		active: {
			...state.active,
			[modelId]: { pct: -1, label }
		}
	});
}

/** Drop an optimistic seed (the invoke failed before any event). */
export function removeActiveDownload(modelId: string): void {
	const state = $downloads.get();
	if (!(modelId in state.active)) return;
	const active = { ...state.active };
	delete active[modelId];
	$downloads.set({ ...state, active });
}

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
 * Side-effect subscribers (the models page's refreshes and toasts) hang
 * off the ONE listener this store owns - no component registers its own
 * model-download listener.
 */
const eventCallbacks = new Set<(event: DownloadEvent) => void>();

export function onDownloadEvent(callback: (event: DownloadEvent) => void): () => void {
	eventCallbacks.add(callback);
	return () => {
		eventCallbacks.delete(callback);
	};
}

/**
 * The feed lives while at least one component reads the store (nanostores
 * onMount: the first subscription registers the listener, the last
 * teardown releases it and clears the state) - the same lifecycle the
 * user and ai-status stores use. With the client router keeping the
 * module scope alive across pages, a subscription never flickers on
 * navigation either.
 */
onMount($downloads, () => {
	// astro's island prerender reads the store at build time (Node, no
	// window and no Tauri IPC): skip registration there; the hydrated
	// webview re-mounts the store and listens for real
	if (typeof window === "undefined") return;
	let stopListening: (() => void) | null = null;
	let disposed = false;
	void listenEvent("modelDownload", (payload) => {
		applyEvent(payload);
		for (const callback of eventCallbacks) {
			callback(payload);
		}
	}).then((unlisten) => {
		// the store lost its last reader before the registration resolved
		if (disposed) {
			unlisten();
		} else {
			stopListening = unlisten;
		}
	});
	return () => {
		disposed = true;
		stopListening?.();
		stopListening = null;
		$downloads.set(IDLE);
	};
});
