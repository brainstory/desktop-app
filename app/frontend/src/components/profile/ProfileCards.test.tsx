import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

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
