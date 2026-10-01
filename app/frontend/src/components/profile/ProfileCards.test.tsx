import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { GeneralCard, timezoneOptions } from "./ProfileCards";

function renderCard(timezone: string) {
	render(<GeneralCard userName="Ada" timezone={timezone} saveSettings={() => {}} />);
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
		render(<GeneralCard userName="Ada" timezone={undefined} saveSettings={() => {}} />);
		expect(screen.getByRole("combobox", { name: "Your timezone" })).toHaveValue("");
	});

	it("saves the detected zone when 'Detect automatically' is chosen", async () => {
		const detected = Intl.DateTimeFormat().resolvedOptions().timeZone;
		const saveSettings = vi.fn();
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
