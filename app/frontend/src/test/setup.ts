import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

// Every component test runs against a mocked Tauri IPC layer: the real
// `invoke` only exists inside the desktop webview.
vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
	// minimal Channel stand-in: capture onmessage, never touch the network
	Channel: class {
		id = -1;
		__internal__: unknown = undefined;
		onmessage: (_event: unknown) => void = () => {};
		toJSON() {
			return "__CHANNEL__:";
		}
	}
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn(async () => () => {})
}));

// jsdom lacks matchMedia (components gate on prefers-reduced-motion)
if (!window.matchMedia) {
	Object.defineProperty(window, "matchMedia", {
		writable: true,
		value: (query: string) => ({
			matches: false,
			media: query,
			onchange: null,
			addListener: () => {},
			removeListener: () => {},
			addEventListener: () => {},
			removeEventListener: () => {},
			dispatchEvent: () => false
		})
	});
}
