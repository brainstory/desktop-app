import { afterEach, describe, expect, it } from "vitest";
import {
	hasDoneGettingStarted,
	markGettingStartedDone,
	setGettingStartedDone
} from "./storage";

describe("storage flags", () => {
	afterEach(() => {
		localStorage.clear();
	});

	it("round-trips the getting-started flag", () => {
		expect(hasDoneGettingStarted()).toBe(false);
		markGettingStartedDone();
		expect(hasDoneGettingStarted()).toBe(true);
	});

	it("is monotonic: clearing is a no-op", () => {
		markGettingStartedDone();
		setGettingStartedDone(false);
		expect(hasDoneGettingStarted()).toBe(true);
	});

	it("tolerates a throwing localStorage", () => {
		const descriptor = Object.getOwnPropertyDescriptor(window, "localStorage");
		Object.defineProperty(window, "localStorage", {
			configurable: true,
			get() {
				throw new Error("storage denied");
			}
		});
		try {
			expect(() => markGettingStartedDone()).not.toThrow();
			expect(hasDoneGettingStarted()).toBe(false);
			expect(() => setGettingStartedDone(true)).not.toThrow();
		} finally {
			if (descriptor) Object.defineProperty(window, "localStorage", descriptor);
		}
	});
});
