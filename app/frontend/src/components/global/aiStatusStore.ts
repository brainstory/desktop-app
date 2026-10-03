import { atom, onMount } from "nanostores";
import { getRuntimeStatusApi, type EngineStatus } from "@helpers/api/models";
import { EVENTS } from "@src/tauri/commands";

/**
 * Single source of truth for the local AI engine states, seeded once
 * and then kept live by the backend's llm-status / stt-status events.
 * Consumers (chat mic gating, dashboard setup card, settings) just
 * `useStore($aiStatus)`: the first subscriber lazily starts the feed
 * (onMount below), so no component has to remember to initialise it.
 */
export interface AiStatus {
	llm: EngineStatus;
	stt: EngineStatus;
	/** false until the backend reported in: llm/stt are placeholders
	 * until then and must not be read as "missing". Optional so code
	 * that sets {llm, stt} directly keeps compiling (absent = loaded). */
	loaded?: boolean;
}

const INITIAL: AiStatus = {
	llm: { state: "missing" },
	stt: { state: "missing" },
	loaded: false
};

export const $aiStatus = atom<AiStatus>(INITIAL);

/** in-flight / finished start; null when not started (or torn down) */
let starting: Promise<void> | null = null;
/** bumped on teardown so a start that is still awaiting bails out */
let generation = 0;
let unlisteners: (() => void)[] = [];
/** pending bounded-retry timer of the current start; null when idle */
let retryTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * Retry budget for a failed listener acquisition or initial read: capped
 * backoff, finitely many attempts, no rapid polling. After the last
 * attempt the store stays "not loaded" until every consumer
 * unsubscribes and a later subscribe starts a fresh lifecycle.
 */
const RETRY_DELAYS_MS = [250, 500, 1000, 2000, 4000, 8000];

function cancelRetry(): void {
	if (retryTimer !== null) {
		clearTimeout(retryTimer);
		retryTimer = null;
	}
}

/**
 * One feed lifecycle. Transient failures — one listen rejecting, or the
 * initial read failing — retry with the bounded backoff above while
 * `gen` is current; teardown (gen bump + cancelRetry) cancels both the
 * timers and the in-flight continuations via the gen checks.
 */
function startLifecycle(gen: number): Promise<void> {
	// per-engine: which engines already reported through a live event.
	// An event is newer than any read answer in flight and must win, but
	// only for the engine that emitted it — one engine's event must not
	// make the other engine's placeholder value authoritative.
	const fromEvent = { llm: false, stt: false };
	let tries = 0;

	const attempt = async (): Promise<void> => {
		if (gen !== generation) return;
		const { listen } = await import("@tauri-apps/api/event");
		if (gen !== generation) return;
		if (unlisteners.length === 0) {
			// Listen BEFORE reading the initial status: an event emitted
			// while the read is in flight is newer than the read's answer
			// and must win. All-or-nothing: if one listen rejects, the
			// sibling that already resolved must be released again.
			const acquired: (() => void)[] = [];
			try {
				acquired.push(
					await listen<EngineStatus>(EVENTS.llmStatus, (event) => {
						fromEvent.llm = true;
						$aiStatus.set({ ...$aiStatus.get(), llm: event.payload });
					})
				);
				acquired.push(
					await listen<EngineStatus>(EVENTS.sttStatus, (event) => {
						fromEvent.stt = true;
						$aiStatus.set({ ...$aiStatus.get(), stt: event.payload });
					})
				);
			} catch (e) {
				acquired.forEach((unlisten) => unlisten());
				throw e;
			}
			if (gen !== generation) {
				acquired.forEach((unlisten) => unlisten());
				return;
			}
			unlisteners = acquired;
		}
		try {
			const status = await getRuntimeStatusApi();
			if (gen !== generation) return;
			const current = $aiStatus.get();
			$aiStatus.set({
				llm: fromEvent.llm ? current.llm : status.llm,
				stt: fromEvent.stt ? current.stt : status.stt,
				loaded: true
			});
		} catch (e) {
			// stays "not loaded": consumers show nothing rather than a
			// wrong "missing"/"loading" claim; live events still update
			// the store and the retry below re-reads
			console.error("failed to read runtime status", e);
			throw e;
		}
	};

	const retry = (): void => {
		if (gen !== generation) return;
		const delay = RETRY_DELAYS_MS[tries];
		if (delay === undefined) return;
		tries++;
		retryTimer = setTimeout(() => {
			retryTimer = null;
			if (gen !== generation) return;
			void attempt().catch(retry);
		}, delay);
	};

	return (async () => {
		try {
			await attempt();
		} catch {
			// the bounded retry above owns recovery; resolving keeps
			// fire-and-forget callers (onMount) free of rejections
			retry();
		}
	})();
}

/** Seed from the backend and subscribe to status events. Idempotent:
 * every caller shares one start, which never rejects — a failed start
 * retries internally with capped backoff. Subscribing to $aiStatus
 * calls this automatically. */
export function initAiStatus(): Promise<void> {
	starting ??= startLifecycle(generation);
	return starting;
}

function teardown(): void {
	generation++;
	starting = null;
	cancelRetry();
	unlisteners.forEach((unlisten) => unlisten());
	unlisteners = [];
	// nobody listened for a while: the values may be stale, re-read on
	// the next subscription
	$aiStatus.set(INITIAL);
}

onMount($aiStatus, () => {
	void initAiStatus();
	return teardown;
});

/**
 * What the chat may do with the language model right now.
 * - unknown: no status yet (don't claim anything)
 * - ready: local model loaded or external endpoint configured
 * - loading: local model is loading
 * - missing: no model downloaded
 * - error: the model failed to load
 */
export type LlmAvailability = "unknown" | "ready" | "loading" | "missing" | "error";

export function llmAvailability(status: AiStatus): LlmAvailability {
	if (status.loaded === false) return "unknown";
	switch (status.llm.state) {
		case "ready":
		case "external":
			return "ready";
		case "loading":
			return "loading";
		case "missing":
			return "missing";
		case "error":
			return "error";
	}
}

/** True only while the local LLM engine is loading (not when it is
 * missing, failed or not yet reported). */
export function llmBusy(status: AiStatus): boolean {
	return llmAvailability(status) === "loading";
}
