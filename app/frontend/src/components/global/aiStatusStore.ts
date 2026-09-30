import { atom } from "nanostores";
import { getRuntimeStatusApi, type EngineStatus } from "@helpers/api/models";

/**
 * Single source of truth for the local AI engine states, fed once at
 * startup and then kept live by the backend's llm-status / stt-status
 * events. Consumers (chat mic gating, dashboard setup card, settings)
 * read the store instead of each polling get_runtime_status
 * themselves.
 */
export interface AiStatus {
	llm: EngineStatus;
	stt: EngineStatus;
}

export const $aiStatus = atom<AiStatus>({
	llm: { state: "missing" },
	stt: { state: "missing" }
});

let started = false;

/** Seed from the backend and subscribe to status events. Safe to call
 * from every consumer; only the first call wires the listeners. */
export async function initAiStatus(): Promise<void> {
	if (started) return;
	started = true;

	try {
		const status = await getRuntimeStatusApi();
		$aiStatus.set(status);
	} catch (e) {
		console.error("failed to read runtime status", e);
	}

	const { listen } = await import("@tauri-apps/api/event");
	const unlistenLlm = await listen<EngineStatus>("llm-status", (event) => {
		$aiStatus.set({ ...$aiStatus.get(), llm: event.payload });
	});
	const unlistenStt = await listen<EngineStatus>("stt-status", (event) => {
		$aiStatus.set({ ...$aiStatus.get(), stt: event.payload });
	});
	void unlistenLlm;
	void unlistenStt;
}

/** True while the local LLM engine is loading or missing entirely. */
export function llmBusy(status: AiStatus): boolean {
	return status.llm.state === "loading" || status.llm.state === "missing";
}
