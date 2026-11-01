import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { EVENTS } from "./commands";
import type { DownloadEvent } from "./generated/DownloadEvent";
import type { EngineStatus } from "./generated/EngineStatus";

/**
 * The payload each backend event carries, keyed by its EVENTS name.
 * Rust-typed payloads use the generated types; quickCaptureBlocked is
 * emitted with no payload at all, and userSettingsChanged's tiny
 * reminder object is declared here until it grows a Rust struct.
 */
export interface EventPayloads {
	llmStatus: EngineStatus;
	sttStatus: EngineStatus;
	modelDownload: DownloadEvent;
	quickCaptureBlocked: null;
	userSettingsChanged: { reminderEnabled: boolean };
}

export type EventName = keyof EventPayloads;

/**
 * Typed `listen`: `name` is an EVENTS key and the handler receives the
 * payload the backend actually emits (see EventPayloads) - not the
 * Event envelope, and not an anything-goes inline type. Import this
 * instead of @tauri-apps/api/event's listen (enforced by lint).
 */
export function listenEvent<K extends EventName>(
	name: K,
	handler: (payload: EventPayloads[K]) => void
): Promise<UnlistenFn> {
	return listen<EventPayloads[K]>(EVENTS[name], (event) => handler(event.payload));
}
