import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";

// the banner only runs inside the Tauri webview (checked at import time)
vi.hoisted(() => {
	Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
});
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import UpdaterBanner from "./UpdaterBanner";

const THROTTLE_KEY = "last_update_check_ms";
/** mirrors the local metadata key the component persists under */
const AVAILABLE_UPDATE_KEY = "updater_available_update";

/** let pending promise chains settle */
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

/** a native Update stand-in; the spies are standalone so tests assert on
 * the locals (method-shaped members trip unbound-method) */
const mockUpdate = (version: string) => {
	const close = vi.fn(async (): Promise<void> => {});
	const downloadAndInstall = vi.fn<Update["downloadAndInstall"]>(async () => {});
	const update = { version, close, downloadAndInstall } as unknown as Update;
	return { update, close, downloadAndInstall };
};

/** simulate arriving on a page after an earlier check found `version` */
const seedAvailable = (version: string) => {
	localStorage.setItem(THROTTLE_KEY, String(Date.now()));
	localStorage.setItem(
		AVAILABLE_UPDATE_KEY,
		JSON.stringify({ version, checkedAtMs: Date.now() })
	);
};

describe("UpdaterBanner", () => {
	beforeEach(() => {
		localStorage.clear();
		vi.mocked(check).mockReset();
		vi.mocked(check).mockResolvedValue(null);
		vi.mocked(relaunch).mockReset();
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
		vi.mocked(check).mockResolvedValue(mockUpdate("9.9.9").update);
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

	it("keeps an available update visible across unmount and remount without a second check", async () => {
		mockInvoke({ get_updates_enabled: () => true });
		const { update: u, close } = mockUpdate("9.9.9");
		vi.mocked(check).mockResolvedValueOnce(u);
		const first = render(<UpdaterBanner />);
		expect(await screen.findByText("Update available: 9.9.9")).toBeInTheDocument();
		first.unmount();
		await flush();
		// the held resource died with the document; it was released once
		expect(close).toHaveBeenCalledTimes(1);
		const invokeCalls = vi.mocked(invoke).mock.calls.length;
		const checkCalls = vi.mocked(check).mock.calls.length;
		// a fresh mount (new page) restores the banner from metadata alone
		render(<UpdaterBanner />);
		expect(await screen.findByText("Update available: 9.9.9")).toBeInTheDocument();
		expect(screen.getByRole("button", { name: "Restart to update" })).toBeInTheDocument();
		await flush();
		// no second network check and no setting read for the restore
		expect(vi.mocked(check).mock.calls.length).toBe(checkCalls);
		expect(vi.mocked(invoke).mock.calls.length).toBe(invokeCalls);
	});

	it("performs no second check when remounting within the throttle after a no-update check", async () => {
		mockInvoke({ get_updates_enabled: () => true });
		vi.mocked(check).mockResolvedValue(null);
		const first = render(<UpdaterBanner />);
		await waitFor(() => expect(check).toHaveBeenCalledTimes(1));
		await flush();
		expect(localStorage.getItem(THROTTLE_KEY)).not.toBeNull();
		first.unmount();
		render(<UpdaterBanner />);
		await flush();
		expect(check).toHaveBeenCalledTimes(1);
		expect(screen.queryByText(/Update available/)).not.toBeInTheDocument();
	});

	it("honors the updates-disabled setting when the install click reacquires", async () => {
		seedAvailable("9.9.9");
		mockInvoke({ get_updates_enabled: () => false });
		render(<UpdaterBanner />);
		const button = await screen.findByRole("button", { name: "Restart to update" });
		// the restored banner made no calls at all so far
		expect(vi.mocked(invoke)).not.toHaveBeenCalled();
		expect(check).not.toHaveBeenCalled();
		fireEvent.click(button);
		await flush();
		// opted out: the reacquire must not contact the update server
		expect(check).not.toHaveBeenCalled();
		expect(screen.getByText("Update failed — try again later.")).toBeInTheDocument();
	});

	it("keeps a failed check retryable on a later mount without consuming the throttle", async () => {
		mockInvoke({ get_updates_enabled: () => true });
		vi.mocked(check).mockRejectedValueOnce(new Error("offline"));
		const first = render(<UpdaterBanner />);
		await waitFor(() => expect(check).toHaveBeenCalledTimes(1));
		await flush();
		// a failed check must not write the throttle
		expect(localStorage.getItem(THROTTLE_KEY)).toBeNull();
		first.unmount();
		vi.mocked(check).mockResolvedValue(null);
		render(<UpdaterBanner />);
		await waitFor(() => expect(check).toHaveBeenCalledTimes(2));
		await flush();
		expect(localStorage.getItem(THROTTLE_KEY)).not.toBeNull();
	});

	it("retries a failed download and resets the downloaded byte count", async () => {
		mockInvoke({ get_updates_enabled: () => true });
		const { update: u, downloadAndInstall } = mockUpdate("9.9.9");
		vi.mocked(check).mockResolvedValue(u);
		let failFirstAttempt!: () => void;
		downloadAndInstall
			.mockImplementationOnce((onEvent) => {
				onEvent?.({ event: "Started", data: { contentLength: 100 } });
				onEvent?.({ event: "Progress", data: { chunkLength: 60 } });
				return new Promise((_resolve, reject) => {
					failFirstAttempt = () => reject(new Error("download failed"));
				});
			})
			.mockImplementationOnce(async (onEvent) => {
				onEvent?.({ event: "Started", data: { contentLength: 100 } });
				onEvent?.({ event: "Progress", data: { chunkLength: 20 } });
				await new Promise(() => {});
			});
		render(<UpdaterBanner />);
		fireEvent.click(await screen.findByRole("button", { name: "Restart to update" }));
		expect(await screen.findByText("Downloading… 60%")).toBeInTheDocument();
		failFirstAttempt();
		expect(await screen.findByText("Update failed — try again later.")).toBeInTheDocument();
		fireEvent.click(screen.getByRole("button", { name: "Try again" }));
		// the byte counter restarted: 20 of 100, not 60 + 20
		expect(await screen.findByText("Downloading… 20%")).toBeInTheDocument();
		// retry reuses the held update; no reacquire check while holding it
		expect(downloadAndInstall).toHaveBeenCalledTimes(2);
		expect(check).toHaveBeenCalledTimes(1);
	});

	it("offers a plain restart instead of a reinstall when the relaunch after install fails", async () => {
		mockInvoke({ get_updates_enabled: () => true });
		const { update: u, downloadAndInstall } = mockUpdate("9.9.9");
		vi.mocked(check).mockResolvedValue(u);
		downloadAndInstall.mockResolvedValue(undefined);
		vi.mocked(relaunch).mockRejectedValueOnce(new Error("relaunch blocked"));
		render(<UpdaterBanner />);
		fireEvent.click(await screen.findByRole("button", { name: "Restart to update" }));
		expect(await screen.findByText(/restart pending/)).toBeInTheDocument();
		expect(downloadAndInstall).toHaveBeenCalledTimes(1);
		fireEvent.click(screen.getByRole("button", { name: "Restart now" }));
		await flush();
		expect(vi.mocked(relaunch)).toHaveBeenCalledTimes(2);
		// the update is already installed: never a second install
		expect(downloadAndInstall).toHaveBeenCalledTimes(1);
	});

	it("closes the held update exactly once when unmounted", async () => {
		mockInvoke({ get_updates_enabled: () => true });
		const { update: u, close } = mockUpdate("9.9.9");
		vi.mocked(check).mockResolvedValue(u);
		const { unmount } = render(<UpdaterBanner />);
		await screen.findByText("Update available: 9.9.9");
		unmount();
		await flush();
		expect(close).toHaveBeenCalledTimes(1);
	});

	it("ignores and closes a late check result that resolves after unmount", async () => {
		mockInvoke({ get_updates_enabled: () => true });
		let resolveCheck!: (u: Update | null) => void;
		vi.mocked(check).mockImplementationOnce(
			() => new Promise<Update | null>((resolve) => (resolveCheck = resolve))
		);
		const { unmount } = render(<UpdaterBanner />);
		await flush();
		unmount();
		const { update: u, close, downloadAndInstall } = mockUpdate("9.9.9");
		resolveCheck(u);
		await flush();
		// the stale result is ignored, and its resource is released once
		expect(close).toHaveBeenCalledTimes(1);
		expect(downloadAndInstall).not.toHaveBeenCalled();
		expect(document.body.textContent).not.toContain("Update available");
	});

	it("clears persisted metadata and hides the banner when the reacquired check finds no update", async () => {
		seedAvailable("9.9.9");
		mockInvoke({ get_updates_enabled: () => true });
		vi.mocked(check).mockResolvedValue(null);
		render(<UpdaterBanner />);
		fireEvent.click(await screen.findByRole("button", { name: "Restart to update" }));
		await waitFor(() => expect(check).toHaveBeenCalledTimes(1));
		await flush();
		expect(screen.queryByText(/Update available/)).not.toBeInTheDocument();
		expect(localStorage.getItem(AVAILABLE_UPDATE_KEY)).toBeNull();
		// the completed reacquire still counts for the throttle
		expect(localStorage.getItem(THROTTLE_KEY)).not.toBeNull();
	});

	it("reacquires a live update on the install click and installs that", async () => {
		seedAvailable("9.9.9");
		mockInvoke({ get_updates_enabled: () => true });
		const { update: u, close, downloadAndInstall } = mockUpdate("9.9.9");
		vi.mocked(check).mockResolvedValue(u);
		downloadAndInstall.mockResolvedValue(undefined);
		vi.mocked(relaunch).mockResolvedValue(undefined);
		render(<UpdaterBanner />);
		// restored from metadata: exactly one user-initiated check, on click
		expect(check).not.toHaveBeenCalled();
		fireEvent.click(await screen.findByRole("button", { name: "Restart to update" }));
		await waitFor(() => expect(check).toHaveBeenCalledTimes(1));
		expect(await screen.findByText("Restarting…")).toBeInTheDocument();
		expect(downloadAndInstall).toHaveBeenCalledTimes(1);
		// the installed update must never be advertised again
		expect(localStorage.getItem(AVAILABLE_UPDATE_KEY)).toBeNull();
		// a successfully installed update needs no close (plugin contract)
		expect(close).not.toHaveBeenCalled();
	});
});
