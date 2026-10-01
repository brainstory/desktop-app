import { describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import Profile from "./Profile";

// The AI models card has its own tests and IPC; keep it out of the way.
vi.mock("./AiModelsCard", () => ({ default: () => null }));

// Capture the daily-log save callback Profile hands its child.
const dailyLog = vi.hoisted(() => ({ save: null as ((ids: number[]) => void) | null }));
vi.mock("./DailyLogSettingsCard", () => ({
	DailyLogSettingsCard: ({ saveSettings }: { saveSettings: (ids: number[]) => void }) => {
		dailyLog.save = saveSettings;
		return null;
	}
}));

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

	it("shows the Updates card with the stored opt-out", async () => {
		mockInvoke({
			get_user_settings: () => ({ ...userSettings, updates: { enabled: false } })
		});
		render(<Profile />);
		expect(
			await screen.findByRole("checkbox", { name: "Check for updates automatically" })
		).not.toBeChecked();
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
		act(() => dailyLog.save!(childState));
		await waitFor(() =>
			expect(vi.mocked(invoke)).toHaveBeenCalledWith("save_user_settings", {
				enabled_log_question_ids: [1, 2, 10]
			})
		);
		expect(childState).toEqual([10, 2, 1]);
	});
});
