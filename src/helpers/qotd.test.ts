import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getQuestionOfTheDay } from "./qotd";

describe("getQuestionOfTheDay", () => {
	beforeEach(() => {
		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
	});

	it("is stable within one local calendar day", () => {
		vi.setSystemTime(new Date(2026, 8, 29, 0, 1));
		const morning = getQuestionOfTheDay();
		vi.setSystemTime(new Date(2026, 8, 29, 12, 0));
		const evening = getQuestionOfTheDay();
		vi.setSystemTime(new Date(2026, 8, 29, 23, 59));
		const night = getQuestionOfTheDay();
		expect(morning).toBe(evening);
		expect(evening).toBe(night);
	});

	it("changes at the local midnight, not 00:00 UTC", () => {
		// 2026-09-29 23:59 local vs 2026-09-30 00:01 local: consecutive
		// local days always map to consecutive question indices
		vi.setSystemTime(new Date(2026, 8, 29, 23, 59));
		const beforeMidnight = getQuestionOfTheDay();
		vi.setSystemTime(new Date(2026, 8, 30, 0, 1));
		const afterMidnight = getQuestionOfTheDay();
		expect(afterMidnight).not.toBe(beforeMidnight);
	});
});
