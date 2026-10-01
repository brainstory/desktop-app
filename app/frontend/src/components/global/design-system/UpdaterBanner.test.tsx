import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";

// the banner only runs inside the Tauri webview (checked at import time)
vi.hoisted(() => {
	Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
});
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

import { check, type Update } from "@tauri-apps/plugin-updater";
import UpdaterBanner from "./UpdaterBanner";

const THROTTLE_KEY = "last_update_check_ms";

/** let pending promise chains settle */
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("UpdaterBanner", () => {
	beforeEach(() => {
		localStorage.clear();
		vi.mocked(check).mockReset();
		vi.mocked(check).mockResolvedValue(null);
	});

	it("never checks for updates when the user turned automatic checks off", async () => {
		mockInvoke({ get_updates_enabled: () => false });
		render(<UpdaterBanner />);
		await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledWith("get_updates_enabled"));
		await flush();
		expect(check).not.toHaveBeenCalled();
		// an opted-out skip is not a completed check
		expect(localStorage.getItem(THROTTLE_KEY)).toBeNull();
	});

	it("checks and offers the update when automatic checks are on", async () => {
		mockInvoke({ get_updates_enabled: () => true });
		vi.mocked(check).mockResolvedValue({ version: "9.9.9" } as Update);
		render(<UpdaterBanner />);
		expect(await screen.findByText("Update available: 9.9.9")).toBeInTheDocument();
		expect(screen.getByRole("button", { name: "Restart to update" })).toBeInTheDocument();
		expect(localStorage.getItem(THROTTLE_KEY)).not.toBeNull();
	});

	it("stays within the 6 hour throttle without reading the setting", async () => {
		localStorage.setItem(THROTTLE_KEY, String(Date.now()));
		mockInvoke({ get_updates_enabled: () => true });
		render(<UpdaterBanner />);
		await flush();
		expect(vi.mocked(invoke)).not.toHaveBeenCalled();
		expect(check).not.toHaveBeenCalled();
	});
});
