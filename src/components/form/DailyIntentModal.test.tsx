import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
function Host({ onClose, setLogId }: { onClose?: () => void; setLogId?: (id: string) => void }) {
	const [open, setOpen] = useState(false);
	return (
		<>
			<button type="button" onClick={() => setOpen(true)}>
				Open log
			</button>
			{open && (
				<DailyIntentModal
					setLogId={setLogId ?? (() => {})}
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

describe("DailyIntentModal load and save failures", () => {
	const questions = [
		{ id: 1, text: "Slept well?", label: "sleep" },
		{ id: 2, text: "Exercised?", label: "exercise" }
	];

	function deferred<T>() {
		let resolve!: (value: T) => void;
		let reject!: (reason?: unknown) => void;
		const promise = new Promise<T>((res, rej) => {
			resolve = res;
			reject = rej;
		});
		return { promise, resolve, reject };
	}

	async function openModal(user: ReturnType<typeof userEvent.setup>) {
		await user.click(screen.getByRole("button", { name: "Open log" }));
	}

	it("shows an accessible error with retry instead of a submittable empty form when the load fails", async () => {
		const user = userEvent.setup();
		const submitLog = vi.fn(() => ({ id: "log1" }));
		mockInvoke({
			get_log_questions: () => Promise.reject(new Error("db locked")),
			submit_log: submitLog
		});
		render(<Host />);
		await openModal(user);

		const alert = await screen.findByRole("alert");
		expect(alert).toHaveTextContent(/couldn.t load your daily questions/i);
		// a failed load must never offer Submit: an empty answer list
		// must not be submittable as if the load had succeeded
		expect(screen.queryByRole("button", { name: "Submit" })).not.toBeInTheDocument();
		expect(submitLog).not.toHaveBeenCalled();
	});

	it("moves focus to the retry button when the load fails", async () => {
		const user = userEvent.setup();
		mockInvoke({
			get_log_questions: () => Promise.reject(new Error("db locked")),
			submit_log: () => ({ id: "log1" })
		});
		render(<Host />);
		await openModal(user);

		expect(await screen.findByRole("button", { name: "Try again" })).toHaveFocus();
	});

	it("recovers when the load retry succeeds and submits the toggled answers", async () => {
		const user = userEvent.setup();
		let calls = 0;
		const submitLog = vi.fn(() => ({ id: "log1" }));
		mockInvoke({
			get_log_questions: () =>
				++calls === 1
					? Promise.reject(new Error("offline"))
					: Promise.resolve({ log: questions }),
			submit_log: submitLog
		});
		render(<Host />);
		await openModal(user);
		await screen.findByRole("alert");

		await user.click(screen.getByRole("button", { name: "Try again" }));
		expect(await screen.findByRole("switch", { name: "Slept well?" })).toBeInTheDocument();
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();

		await user.click(screen.getByRole("switch", { name: "Slept well?" }));
		await user.click(screen.getByRole("button", { name: "Submit" }));

		await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
		// the answer contract is id + value (logItems also carry the
		// question text, which the backend ignores)
		expect(submitLog).toHaveBeenCalledWith({
			log: [
				expect.objectContaining({ id: 1, value: true }),
				expect.objectContaining({ id: 2, value: false })
			]
		});
	});

	it("renders a usable form, not an error, when the question list is legitimately empty", async () => {
		const user = userEvent.setup();
		mockInvoke({
			get_log_questions: () => ({ log: [] }),
			submit_log: () => ({ id: "log1" })
		});
		render(<Host />);
		await openModal(user);

		const submit = await screen.findByRole("button", { name: "Submit" });
		expect(submit).toBeEnabled();
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		expect(screen.queryByRole("switch")).not.toBeInTheDocument();
	});

	it("keeps the entered answers and retries saving after a rejection", async () => {
		const user = userEvent.setup();
		let calls = 0;
		mockInvoke({
			get_log_questions: () => ({ log: questions }),
			submit_log: () =>
				++calls === 1
					? Promise.reject(new Error("disk full"))
					: Promise.resolve({ id: "log1" })
		});
		const onClose = vi.fn();
		const setLogId = vi.fn();
		render(<Host onClose={onClose} setLogId={setLogId} />);
		await openModal(user);

		await user.click(screen.getByRole("switch", { name: "Slept well?" }));
		await user.click(screen.getByRole("button", { name: "Submit" }));

		const alert = await screen.findByRole("alert");
		expect(alert).toHaveTextContent(/couldn.t save your daily log/i);
		// the entered answers survive the rejection
		expect(screen.getByRole("switch", { name: "Slept well?" })).toBeChecked();
		expect(screen.getByRole("switch", { name: "Exercised?" })).not.toBeChecked();
		// Submit is the retry and is usable again
		const submit = screen.getByRole("button", { name: "Submit" });
		expect(submit).toBeEnabled();

		await user.click(submit);
		await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
		expect(onClose).toHaveBeenCalledTimes(1);
		expect(setLogId).toHaveBeenCalledWith("log1");
	});

	it("preserves existing answers returned by the backend, using false only when absent", async () => {
		const user = userEvent.setup();
		mockInvoke({
			get_log_questions: () => ({
				log: [
					{ id: 1, text: "Slept well?", label: "sleep", value: true },
					{ id: 2, text: "Exercised?", label: "exercise", value: false },
					{ id: 3, text: "Stretched?", label: "stretch" }
				]
			}),
			submit_log: () => ({ id: "log1" })
		});
		render(<Host />);
		await openModal(user);

		expect(await screen.findByRole("switch", { name: "Slept well?" })).toBeChecked();
		expect(screen.getByRole("switch", { name: "Exercised?" })).not.toBeChecked();
		expect(screen.getByRole("switch", { name: "Stretched?" })).not.toBeChecked();
	});

	it("ignores a save that settles after the modal closed", async () => {
		const user = userEvent.setup();
		const save = deferred<{ id: string }>();
		const onClose = vi.fn();
		const setLogId = vi.fn();
		mockInvoke({
			get_log_questions: () => ({ log: questions }),
			submit_log: () => save.promise
		});
		render(<Host onClose={onClose} setLogId={setLogId} />);
		await openModal(user);
		await screen.findByRole("switch", { name: "Slept well?" });
		await user.click(screen.getByRole("button", { name: "Submit" }));
		expect(screen.getByRole("button", { name: "Saving" })).toBeDisabled();

		// close while the save is still pending
		await user.keyboard("{Escape}");
		expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
		expect(onClose).toHaveBeenCalledTimes(1);

		// the late success must not re-invoke close or set a log id
		save.resolve({ id: "log1" });
		await new Promise((r) => setTimeout(r, 0));
		expect(onClose).toHaveBeenCalledTimes(1);
		expect(setLogId).not.toHaveBeenCalled();
	});

	it("ignores a load that settles after the modal closed", async () => {
		const user = userEvent.setup();
		const load = deferred<{ log: typeof questions }>();
		const onClose = vi.fn();
		mockInvoke({
			get_log_questions: () => load.promise,
			submit_log: () => ({ id: "log1" })
		});
		render(<Host onClose={onClose} />);
		await openModal(user);
		expect(screen.getByText("Loading your daily questions...")).toBeInTheDocument();

		await user.keyboard("{Escape}");
		expect(onClose).toHaveBeenCalledTimes(1);

		// the late rejection must not update anything or crash
		load.reject(new Error("too late"));
		await new Promise((r) => setTimeout(r, 0));
		expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
		expect(onClose).toHaveBeenCalledTimes(1);
	});

	it("does not submit twice for clicks while a save is pending", async () => {
		const user = userEvent.setup();
		const save = deferred<{ id: string }>();
		const submitLog = vi.fn(() => save.promise);
		mockInvoke({
			get_log_questions: () => ({ log: questions }),
			submit_log: submitLog
		});
		render(<Host />);
		await openModal(user);
		await screen.findByRole("switch", { name: "Slept well?" });

		const submit = screen.getByRole("button", { name: "Submit" });
		fireEvent.click(submit);
		fireEvent.click(submit); // impatient second click while the first is pending
		expect(submitLog).toHaveBeenCalledTimes(1);

		save.resolve({ id: "log1" });
		await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
		expect(submitLog).toHaveBeenCalledTimes(1);
	});
});
