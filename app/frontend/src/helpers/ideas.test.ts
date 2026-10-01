import { describe, expect, it } from "vitest";

import { parseHeadingIndex } from "./ideas";

describe("parseHeadingIndex", () => {
	// sections: [title slot, "# Title", "## A", "## B"]
	const headingCount = 4;

	it("accepts positive ordinals that map to an existing heading", () => {
		expect(parseHeadingIndex("1## Title", headingCount)).toBe(1);
		expect(parseHeadingIndex("3## B", headingCount)).toBe(3);
		expect(parseHeadingIndex(" 2 ##A", headingCount)).toBe(2);
	});

	it.each([
		["empty", ""],
		["null", null],
		["undefined", undefined],
		["no ordinal", "## Heading"],
		["zero (the title slot)", "0## x"],
		["negative", "-1## x"],
		["fraction", "1.5## x"],
		["not a number", "abc## x"],
		["out of range", "4## x"]
	])("rejects %s", (_label, text) => {
		expect(parseHeadingIndex(text, headingCount)).toBeNull();
	});
});
