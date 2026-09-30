import { beforeEach, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

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
	mocked.mockImplementation(((command: string, args?: unknown) => {
		const handler = handlers[command];
		if (!handler) {
			return Promise.reject(new Error(`unexpected invoke("${command}") in test`));
		}
		try {
			return Promise.resolve(handler(args));
		} catch (e) {
			return Promise.reject(e);
		}
	}) as unknown as typeof mocked);
}

beforeEach(() => {
	vi.mocked(invoke).mockReset();
});
