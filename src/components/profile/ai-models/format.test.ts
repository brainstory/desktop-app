import { describe, expect, it } from "vitest";

import { formatEta, formatRate, formatSize } from "./format";

describe("formatSize", () => {
	it("uses GB above a gigabyte and MB below", () => {
		expect(formatSize(1_500_000_000)).toBe("1.5 GB");
		expect(formatSize(2_400_000)).toBe("2 MB");
		expect(formatSize(0)).toBe("");
		expect(formatSize(undefined)).toBe("");
	});
});

describe("formatRate", () => {
	it("reads in MB/s above 1 MB/s, rounded above 100", () => {
		expect(formatRate(12_300_000)).toBe("12.3 MB/s");
		expect(formatRate(250_000_000)).toBe("250 MB/s");
	});
	it("falls back to kB/s for slow transfers", () => {
		expect(formatRate(820_000)).toBe("820 kB/s");
		expect(formatRate(1)).toBe("1 kB/s");
	});
	it("renders nothing without a usable rate", () => {
		expect(formatRate(0)).toBe("");
		expect(formatRate(-5)).toBe("");
	});
});

describe("formatEta", () => {
	it("reads as seconds, minutes or hours", () => {
		expect(formatEta(38)).toBe("38s");
		expect(formatEta(95)).toBe("1m 35s");
		expect(formatEta(600)).toBe("10m 00s");
		expect(formatEta(3720)).toBe("1h 02m");
	});
	it("clamps negative input to zero", () => {
		expect(formatEta(-3)).toBe("0s");
	});
});
