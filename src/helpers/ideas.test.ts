import { describe, expect, it } from "vitest";

import { isOwnIdea, parseHeadingIndex, stripNotesSection, stripResultPreview } from "./ideas";

describe("stripResultPreview", () => {
	it("returns an empty string for missing or non-string input", () => {
		expect(stripResultPreview(undefined)).toBe("");
		expect(stripResultPreview(null)).toBe("");
		expect(stripResultPreview("")).toBe("");
		expect(stripResultPreview(42)).toBe("");
	});

	it("drops the title line before the first blank line", () => {
		expect(stripResultPreview("Title\n\nBody text")).toBe("Body text");
	});

	it("strips a leading ## from the body", () => {
		expect(stripResultPreview("Title\n\n## Section\nMore")).toBe("Section More");
	});

	it("collapses newlines to spaces", () => {
		expect(stripResultPreview("Title\n\nline one\nline two")).toBe("line one line two");
	});

	it("appends the ellipsis only when content is actually cut off", () => {
		const long = "Title\n\n" + "x".repeat(150);
		const preview = stripResultPreview(long);
		expect(preview.endsWith("...")).toBe(true);
		expect(preview.length).toBeLessThan(110);

		const short = "Title\n\nshort body";
		expect(stripResultPreview(short)).toBe("short body");
	});

	it("treats a result without a blank line as the whole body", () => {
		expect(stripResultPreview("just one line")).toBe("just one line");
	});
});

describe("isOwnIdea", () => {
	it("is own exactly when there is no creatorName", () => {
		expect(isOwnIdea({})).toBe(true);
		expect(isOwnIdea({ creatorName: null })).toBe(true);
		expect(isOwnIdea({ creatorName: "" })).toBe(true);
		expect(isOwnIdea({ creatorName: "Ada" })).toBe(false);
	});
});

describe("stripNotesSection", () => {
	it("removes the trailing notes section and nothing else", () => {
		expect(stripNotesSection("# T\n\nbody\n\n## Notes\n\nnote one\n\nnote two")).toBe(
			"# T\n\nbody"
		);
	});

	it("leaves markdown without notes untouched", () => {
		expect(stripNotesSection("# T\n\nbody")).toBe("# T\n\nbody");
	});

	it("handles a document that is only notes", () => {
		expect(stripNotesSection("## Notes\n\nonly a note")).toBe("");
	});

	it("uses the last notes heading, mirroring the backend's rebuild", () => {
		expect(stripNotesSection("a\n\n## Notes\nold")).toBe("a");
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
