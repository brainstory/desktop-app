import { describe, expect, it } from "vitest";
import { isModerationError, normalizeApiError } from "./helpers.js";

describe("normalizeApiError", () => {
	it("passes strings through", () => {
		expect(normalizeApiError("boom")).toBe("boom");
	});

	it("extracts Error messages", () => {
		expect(normalizeApiError(new Error("bad"))).toBe("bad");
	});

	it("stringifies anything else", () => {
		expect(normalizeApiError(42)).toBe("42");
		expect(normalizeApiError(undefined)).toBe("undefined");
	});
});

describe("isModerationError", () => {
	it("matches the exact 469 marker", () => {
		expect(isModerationError("HttpError 469: Inappropriate input")).toBe(true);
	});

	it("matches errors that start with the marker", () => {
		expect(isModerationError("HttpError 469: Inappropriate input (extra detail)")).toBe(true);
	});

	it("matches Error objects carrying the marker", () => {
		expect(isModerationError(new Error("HttpError 469: Inappropriate input"))).toBe(true);
	});

	it("does not match errors that merely quote the marker", () => {
		expect(
			isModerationError('request failed: "HttpError 469: Inappropriate input" echoed back')
		).toBe(false);
	});

	it("does not match unrelated errors", () => {
		expect(isModerationError("model not downloaded")).toBe(false);
	});
});
