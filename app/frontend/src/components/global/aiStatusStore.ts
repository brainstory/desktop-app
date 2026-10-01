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

async function start(gen: number): Promise<void> {
	const { listen } = await import("@tauri-apps/api/event");
	// Listen BEFORE reading the initial status: an event emitted while the
	// read is in flight is newer than the read's answer and must win.
	const fromEvent = { llm: false, stt: false };
	const subscriptions = await Promise.all([
		listen<EngineStatus>(EVENTS.llmStatus, (event) => {
			fromEvent.llm = true;
			$aiStatus.set({ ...$aiStatus.get(), llm: event.payload });
		}),
		listen<EngineStatus>(EVENTS.sttStatus, (event) => {
			fromEvent.stt = true;
			$aiStatus.set({ ...$aiStatus.get(), stt: event.payload });
		})
	]);
	if (gen !== generation) {
		subscriptions.forEach((unlisten) => unlisten());
		return;
	}
	unlisteners = subscriptions;

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
		// stays "not loaded": consumers show nothing rather than a wrong
		// "missing"/"loading" claim; live events still update the store
		console.error("failed to read runtime status", e);
	}
}

/** Seed from the backend and subscribe to status events. Idempotent:
 * every caller shares one start. Subscribing to $aiStatus calls this
 * automatically. */
export function initAiStatus(): Promise<void> {
	starting ??= start(generation);
	return starting;
}

function teardown(): void {
	generation++;
	starting = null;
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
