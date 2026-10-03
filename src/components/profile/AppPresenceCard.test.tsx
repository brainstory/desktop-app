import { describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { mockInvoke } from "@src/test/mock-tauri";
import AppPresenceCard from "./AppPresenceCard";

function deferred() {
	let resolve!: () => void;
	let reject!: (e: unknown) => void;
	const promise = new Promise<void>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

describe("AppPresenceCard", () => {
	it("reverts the switch and reports the error when a save fails", async () => {
		const user = userEvent.setup();
		const openSnackbar = vi.fn();
		mockInvoke({
			set_app_presence: () => {
				throw new Error("could not save");
			}
		});
		render(
			<AppPresenceCard presence={{ dock: true, tray: true }} openSnackbar={openSnackbar} />
		);
		const dock = screen.getByRole("switch", { name: "Show in Dock" });
		await user.click(dock);
		await waitFor(() => expect(openSnackbar).toHaveBeenCalledWith(false, "could not save"));
		await waitFor(() => expect(dock).toHaveAttribute("aria-checked", "true"));
	});

	it("a late failure of an earlier toggle does not undo a later successful one", async () => {
		const user = userEvent.setup();
		const openSnackbar = vi.fn();
		const first = deferred();
		const second = deferred();
		const pending = [first, second];
		mockInvoke({ set_app_presence: () => pending.shift()!.promise });
		render(
			<AppPresenceCard presence={{ dock: true, tray: false }} openSnackbar={openSnackbar} />
		);
		const dock = screen.getByRole("switch", { name: "Show in Dock" });
		const tray = screen.getByRole("switch", { name: "Show in menu bar" });

		await user.click(tray); // save #1: dock on, tray on
		await user.click(dock); // save #2: dock off, tray on
		await act(async () => second.resolve());
		await act(async () => first.reject(new Error("first failed")));

		expect(openSnackbar).toHaveBeenCalledWith(false, "first failed");
		// the backend kept save #2, and so does the UI
		expect(dock).toHaveAttribute("aria-checked", "false");
		expect(tray).toHaveAttribute("aria-checked", "true");
	});

	it("a failed latest toggle falls back to the last saved state, not the original", async () => {
		const user = userEvent.setup();
		const openSnackbar = vi.fn();
		const first = deferred();
		const second = deferred();
		const pending = [first, second];
		mockInvoke({ set_app_presence: () => pending.shift()!.promise });
		render(
			<AppPresenceCard presence={{ dock: true, tray: false }} openSnackbar={openSnackbar} />
		);
		const dock = screen.getByRole("switch", { name: "Show in Dock" });
		const tray = screen.getByRole("switch", { name: "Show in menu bar" });

		await user.click(tray); // save #1: dock on, tray on (succeeds)
		await user.click(dock); // save #2: dock off, tray on (fails)
		await act(async () => second.reject(new Error("second failed")));
		await act(async () => first.resolve());

		expect(dock).toHaveAttribute("aria-checked", "true");
		expect(tray).toHaveAttribute("aria-checked", "true");
	});
});
