import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { SnackbarProvider } from "@ds/Snackbar";
import { QuickCaptureCard } from "./QuickCaptureCard";

function renderCard(enabled: boolean) {
	return render(
		<SnackbarProvider>
			<QuickCaptureCard enabled={enabled} />
		</SnackbarProvider>
	);
}

describe("QuickCaptureCard", () => {
	it("releases the shortcut through set_quick_capture_enabled", async () => {
		const user = userEvent.setup();
		mockInvoke({ set_quick_capture_enabled: () => null });
		renderCard(true);
		const sw = screen.getByRole("switch", { name: "Global shortcut" });
		expect(sw).toHaveAttribute("aria-checked", "true");

		await user.click(sw);
		await waitFor(() =>
			expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_quick_capture_enabled", {
				enabled: false
			})
		);
		expect(sw).toHaveAttribute("aria-checked", "false");
		// the snackbar host confirms the save
		expect(await screen.findByText("Saved")).toBeInTheDocument();
	});

	it("reverts the switch and reports when registering fails", async () => {
		const user = userEvent.setup();
		mockInvoke({
			set_quick_capture_enabled: () => {
				throw new Error("another app already uses it");
			}
		});
		renderCard(false);
		const sw = screen.getByRole("switch", { name: "Global shortcut" });

		await user.click(sw);
		expect(await screen.findByText(/another app already uses it/)).toBeInTheDocument();
		// the failed enable settles back to off
		await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "false"));
	});
});
