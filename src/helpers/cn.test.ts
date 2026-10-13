import { describe, expect, it } from "vitest";
import { cn } from "./cn";

describe("cn", () => {
	it("joins class names with spaces", () => {
		expect(cn("a", "b c")).toBe("a b c");
	});

	it("skips falsy values", () => {
		const empty: string | false = false;
		expect(cn("a", undefined, empty && "b", null, "c")).toBe("a c");
	});

	it("returns an empty string for no input", () => {
		expect(cn()).toBe("");
	});
});
