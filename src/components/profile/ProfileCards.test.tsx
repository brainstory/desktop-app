import { describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { GeneralCard, timezoneOptions } from "./ProfileCards";
import { DailyLogSettingsCard } from "./DailyLogSettingsCard";
import type { LogSettingsQuestion } from "@helpers/api/settings";

/** a save whose outcome the test settles explicitly */
function deferredOutcome() {
	let resolve!: (saved: boolean) => void;
	const promise = new Promise<boolean>((res) => {
		resolve = res;
	});
	return { promise, resolve };
}

function renderCard(timezone: string) {
	render(<GeneralCard userName="Ada" timezone={timezone} saveSettings={async () => true} />);
	return screen.getByRole("combobox", { name: "Your timezone" });
}

describe("GeneralCard timezone select", () => {
	// Intl.supportedValuesOf("timeZone") omits these in Chromium and Node
	it.each(["UTC", "Etc/GMT", "US/Pacific"])(
		"shows a stored %s instead of 'Detect automatically'",
		(stored) => {
			const select = renderCard(stored);
			expect(select).toHaveValue(stored);
			expect(
				screen.getByRole("option", { name: stored.replaceAll("_", " ") })
			).toHaveProperty("selected", true);
		}
	);

	it("keeps canonical zones selectable", () => {
		expect(renderCard("Europe/Berlin")).toHaveValue("Europe/Berlin");
	});

	it("shows 'Detect automatically' when no timezone is stored", () => {
		render(<GeneralCard userName="Ada" timezone={undefined} saveSettings={async () => true} />);
		expect(screen.getByRole("combobox", { name: "Your timezone" })).toHaveValue("");
	});

	it("saves the detected zone when 'Detect automatically' is chosen", async () => {
		const detected = Intl.DateTimeFormat().resolvedOptions().timeZone;
		const saveSettings = vi.fn(async () => true);
		render(<GeneralCard userName="Ada" timezone="Asia/Tokyo" saveSettings={saveSettings} />);
		const user = userEvent.setup();
		await user.selectOptions(
			screen.getByRole("combobox", { name: "Your timezone" }),
			"Detect automatically"
		);
		await user.click(screen.getByRole("button", { name: /save/i }));
		// an empty string was silently dropped by saveUserSettingsApi, so
		// switching back to automatic never persisted anything
		expect(saveSettings).toHaveBeenCalledWith("Ada", detected);
	});
});

describe("timezoneOptions", () => {
	it("always offers UTC, once", () => {
		const zones = timezoneOptions();
		expect(zones.filter((z) => z === "UTC")).toHaveLength(1);
		expect(timezoneOptions("UTC").filter((z) => z === "UTC")).toHaveLength(1);
	});

	it("adds a stored zone the runtime does not list, keeping the list sorted", () => {
		const zones = timezoneOptions("US/Pacific");
		expect(zones).toContain("US/Pacific");
		expect(zones).toEqual([...zones].sort());
	});
});

describe("GeneralCard save outcomes", () => {
	const renderGeneral = (saveSettings: (name: string, timezone: string) => Promise<boolean>) => {
		render(<GeneralCard userName="Ada" timezone="Europe/Berlin" saveSettings={saveSettings} />);
		return {
			nameInput: screen.getByRole("textbox", { name: "Your name" }),
			saveButton: screen.getByRole("button", { name: /save/i })
		};
	};

	it("keeps Save enabled after a failed save so the same values can be retried", async () => {
		const user = userEvent.setup();
		const outcome = deferredOutcome();
		const saveSettings = vi.fn(() => outcome.promise);
		const { nameInput, saveButton } = renderGeneral(saveSettings);

		await user.clear(nameInput);
		await user.type(nameInput, "Bob");
		await user.click(saveButton);
		expect(saveSettings).toHaveBeenCalledWith("Bob", "Europe/Berlin");

		await act(async () => outcome.resolve(false));
		expect(nameInput).toHaveValue("Bob");
		expect(saveButton).toBeEnabled();

		// retry with the SAME values, without editing again
		await user.click(saveButton);
		expect(saveSettings).toHaveBeenCalledTimes(2);
		expect(saveSettings).toHaveBeenLastCalledWith("Bob", "Europe/Berlin");
	});

	it("marks the form clean once a confirmed save completes", async () => {
		const user = userEvent.setup();
		const outcome = deferredOutcome();
		const saveSettings = vi.fn(() => outcome.promise);
		const { nameInput, saveButton } = renderGeneral(saveSettings);

		await user.clear(nameInput);
		await user.type(nameInput, "Bob");
		await user.click(saveButton);
		expect(saveButton).toBeDisabled(); // pending, not clean
		await act(async () => outcome.resolve(true));

		expect(saveSettings).toHaveBeenCalledOnce();
		expect(saveButton).toBeDisabled();
	});

	it("ignores a second click while a save is pending", async () => {
		const user = userEvent.setup();
		const outcome = deferredOutcome();
		const saveSettings = vi.fn(() => outcome.promise);
		const { nameInput, saveButton } = renderGeneral(saveSettings);

		await user.clear(nameInput);
		await user.type(nameInput, "Bob");
		await user.click(saveButton);
		await user.click(saveButton); // still pending
		await act(async () => outcome.resolve(true));

		expect(saveSettings).toHaveBeenCalledOnce();
	});

	it("keeps edits typed during a pending save and leaves them dirty", async () => {
		const user = userEvent.setup();
		const outcome = deferredOutcome();
		const saveSettings = vi.fn(() => outcome.promise);
		const { nameInput, saveButton } = renderGeneral(saveSettings);

		await user.clear(nameInput);
		await user.type(nameInput, "Bob");
		await user.click(saveButton);
		await user.clear(nameInput);
		await user.type(nameInput, "Carol"); // typed while the save was in flight

		await act(async () => outcome.resolve(true));
		expect(nameInput).toHaveValue("Carol");
		// only "Bob" was ever submitted
		expect(saveSettings).toHaveBeenCalledOnce();
		expect(saveSettings).toHaveBeenLastCalledWith("Bob", "Europe/Berlin");
		expect(saveButton).toBeEnabled(); // "Carol" is still unsaved
	});

	it("shows the validation error and does not save an empty name", async () => {
		const user = userEvent.setup();
		const saveSettings = vi.fn(async () => true);
		const { nameInput, saveButton } = renderGeneral(saveSettings);

		await user.clear(nameInput);
		expect(screen.getByText("Name cannot be empty")).toBeInTheDocument();

		await user.click(saveButton);
		expect(saveSettings).not.toHaveBeenCalled();
	});

	it("keeps unsaved edits when the parent passes the saved props back", async () => {
		const user = userEvent.setup();
		const outcome = deferredOutcome();
		const saveSettings = vi.fn(() => outcome.promise);
		const view = render(
			<GeneralCard userName="Ada" timezone="Europe/Berlin" saveSettings={saveSettings} />
		);
		const nameInput = screen.getByRole("textbox", { name: "Your name" });
		const saveButton = screen.getByRole("button", { name: /save/i });

		await user.clear(nameInput);
		await user.type(nameInput, "Bob");
		await user.click(saveButton);
		await user.clear(nameInput);
		await user.type(nameInput, "Carol");
		await act(async () => outcome.resolve(true));

		// the parent refetched after the save; its props now say "Bob"
		view.rerender(
			<GeneralCard userName="Bob" timezone="Europe/Berlin" saveSettings={saveSettings} />
		);
		expect(nameInput).toHaveValue("Carol");
		expect(saveButton).toBeEnabled();
	});
});

describe("DailyLogSettingsCard save outcomes", () => {
	const questions: LogSettingsQuestion[] = [
		{ id: 1, label: "Recovery", questionText: "How did you sleep?", enabled: true },
		{ id: 2, label: "Recovery", questionText: "Did you move your body?", enabled: false }
	];

	const renderDailyLog = (saveSettings: (ids: number[]) => Promise<boolean>) =>
		render(<DailyLogSettingsCard logFieldsData={questions} saveSettings={saveSettings} />);

	it("keeps Save enabled after a failed save so the same ids can be retried", async () => {
		const user = userEvent.setup();
		const outcome = deferredOutcome();
		const saveSettings = vi.fn(() => outcome.promise);
		renderDailyLog(saveSettings);
		const saveButton = screen.getByRole("button", { name: /save/i });
		const moved = screen.getByRole("switch", { name: "Did you move your body?" });

		await user.click(moved); // [1, 2]
		await user.click(saveButton);
		expect(saveSettings).toHaveBeenCalledWith([1, 2]);

		await act(async () => outcome.resolve(false));
		expect(moved).toHaveAttribute("aria-checked", "true");
		expect(saveButton).toBeEnabled();

		// retry with the SAME ids, without toggling again
		await user.click(saveButton);
		expect(saveSettings).toHaveBeenCalledTimes(2);
		expect(saveSettings).toHaveBeenLastCalledWith([1, 2]);
	});

	it("marks the form clean once a confirmed save completes", async () => {
		const user = userEvent.setup();
		const outcome = deferredOutcome();
		const saveSettings = vi.fn(() => outcome.promise);
		renderDailyLog(saveSettings);
		const saveButton = screen.getByRole("button", { name: /save/i });

		await user.click(screen.getByRole("switch", { name: "Did you move your body?" }));
		await user.click(saveButton);
		expect(saveButton).toBeDisabled(); // pending, not clean
		await act(async () => outcome.resolve(true));

		expect(saveSettings).toHaveBeenCalledOnce();
		expect(saveButton).toBeDisabled();
	});

	it("ignores a second click while a save is pending", async () => {
		const user = userEvent.setup();
		const outcome = deferredOutcome();
		const saveSettings = vi.fn(() => outcome.promise);
		renderDailyLog(saveSettings);
		const saveButton = screen.getByRole("button", { name: /save/i });

		await user.click(screen.getByRole("switch", { name: "Did you move your body?" }));
		await user.click(saveButton);
		await user.click(saveButton); // still pending
		await act(async () => outcome.resolve(true));

		expect(saveSettings).toHaveBeenCalledOnce();
	});

	it("keeps toggles made during a pending save and leaves them dirty", async () => {
		const user = userEvent.setup();
		const outcome = deferredOutcome();
		const saveSettings = vi.fn(() => outcome.promise);
		renderDailyLog(saveSettings);
		const saveButton = screen.getByRole("button", { name: /save/i });
		const slept = screen.getByRole("switch", { name: "How did you sleep?" });
		const moved = screen.getByRole("switch", { name: "Did you move your body?" });

		await user.click(moved); // [1, 2]
		await user.click(saveButton); // submits [1, 2]
		await user.click(slept); // [2], toggled while the save was in flight

		await act(async () => outcome.resolve(true));
		expect(saveSettings).toHaveBeenCalledOnce();
		expect(saveSettings).toHaveBeenLastCalledWith([1, 2]);
		expect(slept).toHaveAttribute("aria-checked", "false");
		expect(moved).toHaveAttribute("aria-checked", "true");
		expect(saveButton).toBeEnabled(); // [2] is still unsaved
	});

	it("requires at least one enabled question before saving", async () => {
		const user = userEvent.setup();
		const saveSettings = vi.fn(async () => true);
		renderDailyLog(saveSettings);

		await user.click(screen.getByRole("switch", { name: "How did you sleep?" }));
		await user.click(screen.getByRole("button", { name: /save/i }));

		expect(screen.getByText("Must have at least 1 question enabled")).toBeInTheDocument();
		expect(saveSettings).not.toHaveBeenCalled();
	});

	it("keeps local toggles when the parent passes fresh saved props", async () => {
		const user = userEvent.setup();
		const outcome = deferredOutcome();
		const saveSettings = vi.fn(() => outcome.promise);
		const view = render(
			<DailyLogSettingsCard logFieldsData={questions} saveSettings={saveSettings} />
		);
		const saveButton = screen.getByRole("button", { name: /save/i });
		const slept = screen.getByRole("switch", { name: "How did you sleep?" });
		const moved = screen.getByRole("switch", { name: "Did you move your body?" });

		await user.click(moved); // [1, 2]
		await user.click(saveButton);
		await act(async () => outcome.resolve(true));

		// the parent refetched: the backend now reports only q2 enabled
		const reloaded: LogSettingsQuestion[] = [
			{ ...questions[0]!, enabled: false },
			{ ...questions[1]!, enabled: true }
		];
		view.rerender(
			<DailyLogSettingsCard logFieldsData={reloaded} saveSettings={saveSettings} />
		);
		expect(slept).toHaveAttribute("aria-checked", "true"); // local state, not props
		expect(moved).toHaveAttribute("aria-checked", "true");
	});
});
