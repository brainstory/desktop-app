import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, vi } from "vitest";

// no vitest globals: testing-library cannot self-register its cleanup
afterEach(cleanup);

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

// jsdom's HTMLDialogElement is a bare stub: give it the open/close
// machinery the native-dialog modals use. Focus trapping and
// cancel-on-Escape are browser behaviors of the top layer - tests that
// need the Escape path dispatch the `cancel` event the browser would
// synthesize.
if (typeof HTMLDialogElement !== "undefined" && !HTMLDialogElement.prototype.showModal) {
	HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) {
		this.setAttribute("open", "");
	};
	HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) {
		this.removeAttribute("open");
		this.dispatchEvent(new Event("close"));
	};
}

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
