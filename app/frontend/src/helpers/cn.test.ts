import { describe, expect, it } from "vitest";
import { stripResultPreview } from "./api/user";
import { cn } from "./cn";

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

describe("cn", () => {
	it("joins class names with spaces", () => {
		expect(cn("a", "b c")).toBe("a b c");
	});

	it("skips falsy values", () => {
		expect(cn("a", undefined, false && "b", null, "c")).toBe("a c");
	});

	it("returns an empty string for no input", () => {
		expect(cn()).toBe("");
	});
});
