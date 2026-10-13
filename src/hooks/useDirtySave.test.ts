import { describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";

import { useDirtySave } from "./useDirtySave";

describe("useDirtySave", () => {
	it("is clean at the start and dirty once the value differs", () => {
		const { result } = renderHook(() => useDirtySave("saved"));
		expect(result.current.isDirty("saved")).toBe(false);
		expect(result.current.isDirty("edited")).toBe(true);
	});

	it("a confirmed save moves the clean point to the submitted value", async () => {
		const { result } = renderHook(() => useDirtySave("saved"));
		const persist = vi.fn().mockResolvedValue(true);
		let confirmed = false;
		await act(async () => {
			confirmed = await result.current.save("edited", persist);
		});
		expect(confirmed).toBe(true);
		expect(persist).toHaveBeenCalledWith("edited");
		expect(result.current.saved).toBe("edited");
		expect(result.current.isDirty("edited")).toBe(false);
		expect(result.current.isSaving).toBe(false);
	});

	it("a failed save stays dirty and keeps the old clean point", async () => {
		const { result } = renderHook(() => useDirtySave(1));
		let confirmed = true;
		await act(async () => {
			confirmed = await result.current.save(2, () => Promise.resolve(false));
		});
		expect(confirmed).toBe(false);
		expect(result.current.saved).toBe(1);
		expect(result.current.isDirty(2)).toBe(true);
	});

	it("refuses a second save while one is in flight, and edits made meanwhile stay dirty", async () => {
		const { result } = renderHook(() => useDirtySave("a"));
		let release!: (ok: boolean) => void;
		const inFlight = new Promise<boolean>((resolve) => {
			release = resolve;
		});
		let first!: Promise<boolean>;
		act(() => {
			first = result.current.save("b", () => inFlight);
		});
		expect(result.current.isSaving).toBe(true);

		// a second activation in the same tick is a no-op, not a second save
		let second = true;
		await act(async () => {
			second = await result.current.save("c", () => Promise.resolve(true));
		});
		expect(second).toBe(false);

		await act(async () => {
			release(true);
			await first;
		});
		// the confirmed value is the FIRST save's payload...
		expect(result.current.saved).toBe("b");
		// ...so the edit that landed during the save is still unsaved
		expect(result.current.isDirty("c")).toBe(true);
		expect(result.current.isSaving).toBe(false);
	});

	it("uses a caller-supplied equality for structural state", () => {
		const sameOrder = (a: string[], b: string[]) => a.join() === b.join();
		const { result } = renderHook(() => useDirtySave(["one", "two"], sameOrder));
		// a different array with the same contents is not dirty
		expect(result.current.isDirty(["one", "two"])).toBe(false);
		expect(result.current.isDirty(["two", "one"])).toBe(true);
	});
});
