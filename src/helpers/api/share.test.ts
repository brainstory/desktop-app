import { describe, expect, it } from "vitest";

import { shortenHomePath } from "./share";

describe("shortenHomePath", () => {
	it("shortens a leading home directory to ~", () => {
		expect(shortenHomePath("/Users/ada/Downloads/brainstory-idea.md")).toBe(
			"~/Downloads/brainstory-idea.md"
		);
		expect(shortenHomePath("C:\\Users\\ada\\Documents\\note.md")).toBe("~\\Documents\\note.md");
	});

	it("leaves paths outside the home directory alone", () => {
		expect(shortenHomePath("/tmp/export.md")).toBe("/tmp/export.md");
		expect(shortenHomePath("/Volumes/USB/brainstory.md")).toBe("/Volumes/USB/brainstory.md");
	});
});
