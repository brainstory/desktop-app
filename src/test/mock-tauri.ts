import { beforeEach, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen, type EventCallback } from "@tauri-apps/api/event";

/**
 * Route Tauri IPC by command name for one test:
 *
 *   mockInvoke({ get_idea: () => ({ id: "i1" }) });
 *
 * Unhandled commands reject with a descriptive error, and unexpected
 * calls after the mocks are set fail the test. Call it in
 * beforeEach/beforeAll; state resets with vi.clearAllMocks.
 */
export function mockInvoke(handlers: Record<string, (args: unknown) => unknown>): void {
	const mocked = vi.mocked(invoke);
	mocked.mockImplementation((command: string, args?: unknown) => {
		const handler = handlers[command];
		if (!handler) {
			return Promise.reject(new Error(`unexpected invoke("${command}") in test`));
		}
		try {
			return Promise.resolve(handler(args));
		} catch (e) {
			return Promise.reject(e);
		}
	});
}

/** The mocked `listen` (setup.ts mocks the module away); configure it
 * with vi.mocked semantics in tests that need it. */
export const mockedListen = vi.mocked(listen);

/** Capture the handlers registered through `listen` during a test and
 * emit fake backend events to them. */
export function captureListen(): {
	handlers: EventCallback<unknown>[];
	emit: (event: string, payload: unknown) => void;
} {
	const handlers: EventCallback<unknown>[] = [];
	vi.mocked(listen).mockImplementation(async (_event, handler) => {
		handlers.push(handler);
		return () => {};
	});
	return {
		handlers,
		emit: (event, payload) => {
			for (const handler of handlers) {
				handler({ event, id: 0, payload });
			}
		}
	};
}

beforeEach(() => {
	vi.mocked(invoke).mockReset();
});
