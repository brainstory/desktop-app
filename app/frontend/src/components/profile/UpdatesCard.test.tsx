import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import UpdatesCard from "./UpdatesCard";

const WARNING = /Automatic update checks are off/;

describe("UpdatesCard", () => {
	it("renders the checkbox checked by default without a warning", () => {
		render(<UpdatesCard enabled={true} openSnackbar={vi.fn()} />);
		expect(
			screen.getByRole("checkbox", { name: "Check for updates automatically" })
		).toBeChecked();
		expect(screen.queryByText(WARNING)).not.toBeInTheDocument();
	});

	it("unchecking saves the opt-out and shows the warning", async () => {
		const user = userEvent.setup();
		mockInvoke({ set_updates_enabled: () => undefined });
		render(<UpdatesCard enabled={true} openSnackbar={vi.fn()} />);
		const checkbox = screen.getByRole("checkbox", { name: "Check for updates automatically" });
		await user.click(checkbox);
		expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_updates_enabled", { enabled: false });
		expect(checkbox).not.toBeChecked();
		expect(screen.getByRole("status")).toHaveTextContent(
			"Automatic update checks are off. You could be missing new features, bug fixes and security fixes."
		);
	});

	it("reverts and reports the error when saving fails", async () => {
		const user = userEvent.setup();
		const openSnackbar = vi.fn();
		mockInvoke({
			set_updates_enabled: () => {
				throw new Error("could not save");
			}
		});
		render(<UpdatesCard enabled={true} openSnackbar={openSnackbar} />);
		const checkbox = screen.getByRole("checkbox", { name: "Check for updates automatically" });
		await user.click(checkbox);
		await waitFor(() => expect(openSnackbar).toHaveBeenCalledWith(false, "could not save"));
		await waitFor(() => expect(checkbox).toBeChecked());
		expect(screen.queryByText(WARNING)).not.toBeInTheDocument();
	});

	it("shows the warning straight away when updates were already off", () => {
		render(<UpdatesCard enabled={false} openSnackbar={vi.fn()} />);
		expect(
			screen.getByRole("checkbox", { name: "Check for updates automatically" })
		).not.toBeChecked();
		expect(screen.getByText(WARNING)).toBeInTheDocument();
	});
});
