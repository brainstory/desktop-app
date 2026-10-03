import { describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import Profile from "./Profile";

// The AI models card has its own tests and IPC; keep it out of the way.
vi.mock("./AiModelsCard", () => ({ default: () => null }));

// Capture the daily-log save callback Profile hands its child.
const dailyLog = vi.hoisted(() => ({
	save: null as ((ids: number[]) => Promise<boolean>) | null
}));
vi.mock("./DailyLogSettingsCard", () => ({
	DailyLogSettingsCard: ({
		saveSettings
	}: {
		saveSettings: (ids: number[]) => Promise<boolean>;
	}) => {
		dailyLog.save = saveSettings;
		return null;
	}
}));

/** a backend call whose outcome the test settles explicitly */
function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (e: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

const userSettings = {
	user: { name: "Ada", timezone: "Europe/Berlin" },
	log: [],
	notifications: [],
	presence: { dock: true, tray: true },
	updates: { enabled: true }
};

describe("Profile", () => {
	it("shows an error with a working retry when settings fail to load", async () => {
		const user = userEvent.setup();
		let calls = 0;
		mockInvoke({
			get_user_settings: () => {
				calls += 1;
				if (calls === 1) throw new Error("db locked");
				return userSettings;
			}
		});
		render(<Profile />);
		expect(await screen.findByText("Couldn't load your settings")).toBeInTheDocument();

		await user.click(screen.getByRole("button", { name: "Try again" }));
		expect(await screen.findByText("General Information")).toBeInTheDocument();
		expect(screen.queryByText("Couldn't load your settings")).not.toBeInTheDocument();
		expect(calls).toBe(2);
	});

	it("shows 'Detect automatically' rather than Etc/GMT when no timezone is stored", async () => {
		mockInvoke({
			get_user_settings: () => ({ ...userSettings, user: { name: "Ada" } })
		});
		render(<Profile />);
		expect(await screen.findByRole("combobox", { name: "Your timezone" })).toHaveValue("");
	});

	it("labels the reminder hour with the stored timezone", async () => {
		mockInvoke({
			get_user_settings: () => ({
				...userSettings,
				notifications: [
					{
						title: "Daily intention reminder",
						description: "A daily reminder.",
						value: "09:00",
						value_type: "time",
						enabled: false
					}
				]
			})
		});
		render(<Profile />);
		// the hour the picker edits is wall-clock time in the stored zone
		expect(await screen.findByText("Europe/Berlin time")).toBeInTheDocument();
	});

	it("shows the Updates card with the stored opt-out", async () => {
		mockInvoke({
			get_user_settings: () => ({ ...userSettings, updates: { enabled: false } })
		});
		render(<Profile />);
		expect(
			await screen.findByRole("switch", { name: "Check for updates automatically" })
		).toHaveAttribute("aria-checked", "false");
		expect(screen.getByText(/Automatic update checks are off/)).toBeInTheDocument();
	});

	it("stacks App Presence and Updates as full-width rows, not side by side", async () => {
		mockInvoke({ get_user_settings: () => userSettings });
		render(<Profile />);
		for (const title of ["App Presence", "Updates"]) {
			const card = (await screen.findByRole("heading", { name: title })).closest(
				"div.w-full"
			);
			// a full-width card in the 3-column grid, one per row
			expect(card?.className).toContain("lg:col-span-3");
		}
	});

	it("shows the updates-off warning above the tabs, on every tab", async () => {
		const user = userEvent.setup();
		mockInvoke({
			get_user_settings: () => ({ ...userSettings, updates: { enabled: false } })
		});
		render(<Profile />);
		const warning = await screen.findByText(/Automatic update checks are off/);
		// rendered before the tab list in document order, not inside a card
		expect(
			warning.compareDocumentPosition(screen.getByRole("tablist")) &
				Node.DOCUMENT_POSITION_FOLLOWING
		).toBeTruthy();
		await user.click(screen.getByRole("tab", { name: "Daily Log Settings" }));
		expect(screen.getByText(/Automatic update checks are off/)).toBeInTheDocument();
	});

	it("saves daily log ids sorted numerically without touching the child's array", async () => {
		const user = userEvent.setup();
		mockInvoke({
			get_user_settings: () => userSettings,
			save_user_settings: () => ({ id: "settings" })
		});
		render(<Profile />);
		await user.click(await screen.findByRole("tab", { name: "Daily Log Settings" }));
		await waitFor(() => expect(dailyLog.save).not.toBeNull());

		// frozen: an in-place sort of the child's state would throw
		const childState = Object.freeze([10, 2, 1]) as number[];
		await act(async () => {
			await dailyLog.save!(childState);
		});
		await waitFor(() =>
			expect(vi.mocked(invoke)).toHaveBeenCalledWith("save_user_settings", {
				enabled_log_question_ids: [1, 2, 10]
			})
		);
		expect(childState).toEqual([10, 2, 1]);
	});

	it("keeps the General form editable and retryable when a save fails", async () => {
		const user = userEvent.setup();
		// an earlier test's tab click left ?tab= in the shared jsdom URL
		window.history.replaceState(null, "", "/");
		const failed = deferred<{ id: string }>();
		const ok = deferred<{ id: string }>();
		const saves = [failed, ok];
		let loadCount = 0;
		mockInvoke({
			get_user_settings: () => {
				loadCount += 1;
				return loadCount === 1
					? userSettings
					: { ...userSettings, user: { name: "Bob", timezone: "Europe/Berlin" } };
			},
			save_user_settings: () => saves.shift()!.promise
		});
		render(<Profile />);
		const nameInput = await screen.findByRole("textbox", { name: "Your name" });
		const saveButton = screen.getByRole("button", { name: /save/i });

		await user.clear(nameInput);
		await user.type(nameInput, "Bob");
		await user.click(saveButton);
		await act(async () => failed.reject(new Error("save failed")));

		// error surfaced, unsaved value kept, Save still retryable
		expect(await screen.findByText("save failed")).toBeInTheDocument();
		expect(nameInput).toHaveValue("Bob");
		expect(saveButton).toBeEnabled();

		await user.click(saveButton); // retry with the SAME values
		await act(async () => ok.resolve({ id: "settings" }));
		expect(await screen.findByText("Changes Saved!")).toBeInTheDocument();
		expect(nameInput).toHaveValue("Bob");
		expect(saveButton).toBeDisabled();
	});

	it("preserves edits typed during a pending General save when reloaded settings arrive", async () => {
		const user = userEvent.setup();
		// an earlier test's tab click left ?tab= in the shared jsdom URL
		window.history.replaceState(null, "", "/");
		const save = deferred<{ id: string }>();
		let loadCount = 0;
		mockInvoke({
			get_user_settings: () => {
				loadCount += 1;
				return loadCount === 1
					? userSettings
					: { ...userSettings, user: { name: "Bob", timezone: "Europe/Berlin" } };
			},
			save_user_settings: () => save.promise
		});
		render(<Profile />);
		const nameInput = await screen.findByRole("textbox", { name: "Your name" });
		const saveButton = screen.getByRole("button", { name: /save/i });

		await user.clear(nameInput);
		await user.type(nameInput, "Bob");
		await user.click(saveButton);
		await user.clear(nameInput);
		await user.type(nameInput, "Carol"); // typed while the save was in flight
		await act(async () => save.resolve({ id: "settings" }));

		// two loads + one save; the reload delivered the saved props
		await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledTimes(3));
		expect(nameInput).toHaveValue("Carol"); // not clobbered by "Bob"
		expect(saveButton).toBeEnabled(); // "Carol" is still unsaved
	});

	it("resolves the daily log save with false and reports the error, then true on retry", async () => {
		const user = userEvent.setup();
		// start from the General tab regardless of earlier tab clicks
		window.history.replaceState(null, "", "/");
		let calls = 0;
		mockInvoke({
			get_user_settings: () => userSettings,
			save_user_settings: () => {
				calls += 1;
				if (calls === 1) throw new Error("log save failed");
				return { id: "settings" };
			}
		});
		render(<Profile />);
		await user.click(await screen.findByRole("tab", { name: "Daily Log Settings" }));
		await waitFor(() => expect(dailyLog.save).not.toBeNull());

		let outcome: boolean | undefined;
		await act(async () => {
			outcome = await dailyLog.save!([2, 1]);
		});
		expect(outcome).toBe(false);
		expect(await screen.findByText("log save failed")).toBeInTheDocument();

		let retry: boolean | undefined;
		await act(async () => {
			retry = await dailyLog.save!([1, 2]);
		});
		expect(retry).toBe(true);
		expect(await screen.findByText("Changes Saved!")).toBeInTheDocument();
	});
});
