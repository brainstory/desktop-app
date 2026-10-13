import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { QuickCaptureCard } from "./QuickCaptureCard";

describe("QuickCaptureCard", () => {
	it("releases the shortcut through set_quick_capture_enabled", async () => {
		const user = userEvent.setup();
		mockInvoke({ set_quick_capture_enabled: () => null });
		const openSnackbar = vi.fn();
		render(<QuickCaptureCard enabled={true} openSnackbar={openSnackbar} />);
		const sw = screen.getByRole("switch", { name: "Global shortcut" });
		expect(sw).toHaveAttribute("aria-checked", "true");

		await user.click(sw);
		await waitFor(() =>
			expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_quick_capture_enabled", {
				enabled: false
			})
		);
		expect(sw).toHaveAttribute("aria-checked", "false");
		expect(openSnackbar).toHaveBeenCalledWith(true, "Saved");
	});

	it("reverts the switch and reports when registering fails", async () => {
		const user = userEvent.setup();
		mockInvoke({
			set_quick_capture_enabled: () => {
				throw new Error("another app already uses it");
			}
		});
		const openSnackbar = vi.fn();
		render(<QuickCaptureCard enabled={false} openSnackbar={openSnackbar} />);
		const sw = screen.getByRole("switch", { name: "Global shortcut" });

		await user.click(sw);
		await waitFor(() =>
			expect(openSnackbar).toHaveBeenCalledWith(
				false,
				expect.stringContaining("another app already uses it")
			)
		);
		// the failed enable settles back to off
		await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "false"));
	});
});
