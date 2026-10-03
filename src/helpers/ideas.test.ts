import { describe, expect, it } from "vitest";

import { isOwnIdea, parseHeadingIndex } from "./ideas";

describe("isOwnIdea", () => {
	it("is own exactly when there is no creatorName", () => {
		expect(isOwnIdea({})).toBe(true);
		expect(isOwnIdea({ creatorName: null })).toBe(true);
		expect(isOwnIdea({ creatorName: "" })).toBe(true);
		expect(isOwnIdea({ creatorName: "Ada" })).toBe(false);
	});
});

describe("parseHeadingIndex", () => {
	// result sections as the backend returns them: [title slot, "# Title", "## A", "## B"]
	const sections = [
		{ heading: "" },
		{ heading: "# Title" },
		{ heading: "## A" },
		{ heading: "## B" }
	];

	it("numbers only the ## sections, as the feedback prompt and the model do", () => {
		// "1##" is the FIRST ## section, not the # title: using the number as
		// a raw array index put every comment one section too early
		expect(parseHeadingIndex("1## x", sections)).toBe(2);
		expect(parseHeadingIndex("2## y", sections)).toBe(3);
	});

	it("places a comment by its heading text when it names a section", () => {
		expect(parseHeadingIndex("1## B", sections)).toBe(3);
		expect(parseHeadingIndex(" 2 ##A", sections)).toBe(2);
		expect(parseHeadingIndex("1## title", sections)).toBe(1);
		expect(parseHeadingIndex("## B", sections)).toBe(3);
	});

	it("prefers the ## section when the title and a section share their text", () => {
		const twin = [{ heading: "" }, { heading: "# Focus" }, { heading: "## Focus" }];
		expect(parseHeadingIndex("1## Focus", twin)).toBe(2);
	});

	it.each([
		["empty", ""],
		["null", null],
		["undefined", undefined],
		["no ordinal and unknown text", "## Nowhere"],
		["zero", "0## x"],
		["negative", "-1## x"],
		["fraction", "1.5## x"],
		["not a number", "abc## x"],
		["past the last ## section", "3## x"]
	])("rejects %s", (_label, text) => {
		expect(parseHeadingIndex(text, sections)).toBeNull();
	});
});
