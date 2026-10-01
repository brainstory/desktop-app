import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";

import { mockInvoke } from "@src/test/mock-tauri";
import DailyIntentModal from "./DailyIntentModal";

beforeEach(() => {
	mockInvoke({
		get_log_questions: () => ({
			log: [
				{ id: 1, text: "Slept well?", label: "sleep" },
				{ id: 2, text: "Exercised?", label: "exercise" }
			]
		}),
		submit_log: () => ({ id: "log1" })
	});
});

/** An opener button outside the modal, like the real chat page. */
function Host({ onClose }: { onClose?: () => void }) {
	const [open, setOpen] = useState(false);
	return (
		<>
			<button type="button" onClick={() => setOpen(true)}>
				Open log
			</button>
			{open && (
				<DailyIntentModal
					setLogId={() => {}}
					onClose={() => {
						onClose?.();
						setOpen(false);
					}}
				/>
			)}
		</>
	);
}

describe("DailyIntentModal", () => {
	it("moves focus into the dialog and traps Tab inside it", async () => {
		const user = userEvent.setup();
		render(<Host />);
		await user.click(screen.getByRole("button", { name: "Open log" }));
		const dialog = screen.getByRole("dialog", { name: "Daily Intent Log" });
		expect(dialog).toContainElement(document.activeElement as HTMLElement);

		// wait for the questions so the dialog has all its controls
		await screen.findByText("Slept well?");
		for (let i = 0; i < 8; i++) {
			await user.tab();
			expect(dialog).toContainElement(document.activeElement as HTMLElement);
		}
		for (let i = 0; i < 8; i++) {
			await user.tab({ shift: true });
			expect(dialog).toContainElement(document.activeElement as HTMLElement);
		}
	});

	it("closes on Escape", async () => {
		const user = userEvent.setup();
		const onClose = vi.fn();
		render(<Host onClose={onClose} />);
		await user.click(screen.getByRole("button", { name: "Open log" }));
		await screen.findByText("Slept well?");
		await user.keyboard("{Escape}");
		expect(onClose).toHaveBeenCalledTimes(1);
		expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
	});

	it("returns focus to the element that opened it", async () => {
		const user = userEvent.setup();
		render(<Host />);
		const opener = screen.getByRole("button", { name: "Open log" });
		await user.click(opener);
		await screen.findByText("Slept well?");
		await user.click(screen.getByRole("button", { name: "Close modal" }));
		expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
		expect(opener).toHaveFocus();
	});

	it("labels each answer switch with its question", async () => {
		const user = userEvent.setup();
		render(<Host />);
		await user.click(screen.getByRole("button", { name: "Open log" }));
		expect(await screen.findByRole("switch", { name: "Slept well?" })).toBeInTheDocument();
		expect(screen.getByRole("switch", { name: "Exercised?" })).toBeInTheDocument();
	});
});
