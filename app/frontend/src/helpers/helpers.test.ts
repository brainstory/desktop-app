import { afterEach, describe, expect, it, vi } from "vitest";
import {
	callApiWithRetry,
	formatISO8601ToHumanReadable,
	isModerationError,
	normalizeApiError,
	parseBackendUtc
} from "./helpers";
import { isTransientTranscriptionError } from "@components/recording-ui/transcription";

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

describe("parseBackendUtc", () => {
	it("treats naive timestamps as UTC", () => {
		const parsed = parseBackendUtc("2026-09-15T10:30:00");
		expect(parsed?.toISOString()).toBe("2026-09-15T10:30:00.000Z");
	});

	it("keeps strings that already carry a zone", () => {
		expect(parseBackendUtc("2026-09-15T10:30:00+02:00")?.toISOString()).toBe(
			"2026-09-15T08:30:00.000Z"
		);
		expect(parseBackendUtc("2026-09-15T10:30:00Z")?.toISOString()).toBe(
			"2026-09-15T10:30:00.000Z"
		);
	});

	it("returns null for empty or unparseable input", () => {
		expect(parseBackendUtc("")).toBeNull();
		expect(parseBackendUtc(null)).toBeNull();
		expect(parseBackendUtc(undefined)).toBeNull();
		expect(parseBackendUtc("   ")).toBeNull();
		expect(parseBackendUtc("yesterday")).toBeNull();
	});
});

describe("formatISO8601ToHumanReadable", () => {
	it("renders a placeholder instead of Invalid Date for null-ish input", () => {
		expect(formatISO8601ToHumanReadable("")).toBe("—");
		expect(formatISO8601ToHumanReadable("garbage")).toBe("—");
	});

	it("formats naive UTC timestamps in the local zone", () => {
		// compare against the same conversion done by the platform, so the
		// assertion holds in any runner timezone
		const expected = new Date("2026-09-15T10:30:00Z").toLocaleDateString("en-US", {
			year: "numeric",
			month: "short",
			day: "numeric",
			hour: "numeric",
			minute: "2-digit"
		});
		expect(formatISO8601ToHumanReadable("2026-09-15T10:30:00")).toBe(expected);
	});
});

describe("callApiWithRetry", () => {
	afterEach(() => {
		vi.useRealTimers();
	});

	it("retries once after 500 ms and resolves on the second attempt", async () => {
		vi.useFakeTimers();
		let calls = 0;
		const promise = callApiWithRetry(async () => {
			calls++;
			if (calls === 1) throw new Error("transient");
			return "ok";
		});
		await vi.advanceTimersByTimeAsync(500);
		await expect(promise).resolves.toBe("ok");
		expect(calls).toBe(2);
	});

	it("does not retry moderation errors", async () => {
		vi.useFakeTimers();
		let calls = 0;
		const promise = callApiWithRetry(async () => {
			calls++;
			throw new Error("HttpError 469: Inappropriate input");
		});
		promise.catch(() => {});
		await vi.advanceTimersByTimeAsync(5000);
		await expect(promise).rejects.toThrow("469");
		expect(calls).toBe(1);
	});

	it("rejects with the last error after exhausting retries", async () => {
		vi.useFakeTimers();
		let calls = 0;
		const promise = callApiWithRetry(async () => {
			calls++;
			throw new Error("still broken");
		});
		promise.catch(() => {});
		await vi.advanceTimersByTimeAsync(5000);
		await expect(promise).rejects.toThrow("still broken");
		expect(calls).toBe(2);
	});

	it("does not retry when the predicate says the error is permanent", async () => {
		vi.useFakeTimers();
		let calls = 0;
		// the real transcription predicate RecordButton uses, not a copy
		const promise = callApiWithRetry(
			async () => {
				calls++;
				throw new Error("Speech model not downloaded yet.");
			},
			1,
			isTransientTranscriptionError
		);
		promise.catch(() => {});
		await vi.advanceTimersByTimeAsync(5000);
		await expect(promise).rejects.toThrow("not downloaded yet");
		expect(calls).toBe(1);
	});
});
