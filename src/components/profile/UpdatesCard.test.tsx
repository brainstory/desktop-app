import { useEffect } from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { UpdatesCard, UpdatesDisabledWarning, useUpdatesEnabled } from "./UpdatesCard";

const WARNING = /Automatic update checks are off/;

/** The settings page's wiring: one state feeding the card and the warning. */
function Harness({
	stored,
	openSnackbar = vi.fn()
}: {
	stored: boolean;
	openSnackbar?: (isSuccess: boolean, message: string) => void;
}) {
	const updates = useUpdatesEnabled(openSnackbar);
	const { load } = updates;
	useEffect(() => load(stored), [load, stored]);
	return (
		<>
			<UpdatesDisabledWarning enabled={updates.enabled} onTurnOn={() => updates.save(true)} />
			<UpdatesCard enabled={updates.enabled} onToggle={updates.save} />
		</>
	);
}

const toggle = () => screen.getByRole("switch", { name: "Check for updates automatically" });

describe("Updates setting", () => {
	it("renders the switch on by default without a warning", () => {
		render(<Harness stored={true} />);
		expect(toggle()).toHaveAttribute("aria-checked", "true");
		expect(screen.queryByText(WARNING)).not.toBeInTheDocument();
	});

	it("switching off saves the opt-out and shows the warning", async () => {
		const user = userEvent.setup();
		mockInvoke({ set_updates_enabled: () => undefined });
		render(<Harness stored={true} />);
		await user.click(toggle());
		expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_updates_enabled", { enabled: false });
		expect(toggle()).toHaveAttribute("aria-checked", "false");
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
		render(<Harness stored={true} openSnackbar={openSnackbar} />);
		await user.click(toggle());
		await waitFor(() => expect(openSnackbar).toHaveBeenCalledWith(false, "could not save"));
		await waitFor(() => expect(toggle()).toHaveAttribute("aria-checked", "true"));
		expect(screen.queryByText(WARNING)).not.toBeInTheDocument();
	});

	it("shows the warning straight away when updates were already off", () => {
		render(<Harness stored={false} />);
		expect(toggle()).toHaveAttribute("aria-checked", "false");
		expect(screen.getByText(WARNING)).toBeInTheDocument();
	});

	it("'Turn back on' in the warning re-enables update checks", async () => {
		const user = userEvent.setup();
		mockInvoke({ set_updates_enabled: () => undefined });
		render(<Harness stored={false} />);
		await user.click(screen.getByRole("button", { name: "Turn back on" }));
		expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_updates_enabled", { enabled: true });
		expect(toggle()).toHaveAttribute("aria-checked", "true");
		expect(screen.queryByText(WARNING)).not.toBeInTheDocument();
	});
});
