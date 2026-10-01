import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";

import { useConfirmClick, useTimeout } from "./useTimeout";

beforeEach(() => {
	vi.useFakeTimers();
});

afterEach(() => {
	vi.useRealTimers();
});

describe("useTimeout", () => {
	it("runs the scheduled callback after the delay", () => {
		const fn = vi.fn();
		const { result } = renderHook(() => useTimeout());
		act(() => result.current[0](fn, 1000));
		act(() => vi.advanceTimersByTime(999));
		expect(fn).not.toHaveBeenCalled();
		act(() => vi.advanceTimersByTime(1));
		expect(fn).toHaveBeenCalledTimes(1);
	});

	it("re-scheduling replaces the pending timer", () => {
		const first = vi.fn();
		const second = vi.fn();
		const { result } = renderHook(() => useTimeout());
		act(() => result.current[0](first, 1000));
		act(() => result.current[0](second, 1000));
		act(() => vi.advanceTimersByTime(1000));
		expect(first).not.toHaveBeenCalled();
		expect(second).toHaveBeenCalledTimes(1);
	});

	it("cancel stops the pending timer", () => {
		const fn = vi.fn();
		const { result } = renderHook(() => useTimeout());
		act(() => result.current[0](fn, 1000));
		act(() => result.current[1]());
		act(() => vi.advanceTimersByTime(5000));
		expect(fn).not.toHaveBeenCalled();
	});

	it("clears the timer on unmount", () => {
		const fn = vi.fn();
		const { result, unmount } = renderHook(() => useTimeout());
		act(() => result.current[0](fn, 1000));
		unmount();
		act(() => vi.advanceTimersByTime(5000));
		expect(fn).not.toHaveBeenCalled();
		expect(vi.getTimerCount()).toBe(0);
	});

	it("returns stable identities across renders", () => {
		const { result, rerender } = renderHook(() => useTimeout());
		const [schedule, cancel] = result.current;
		rerender();
		expect(result.current[0]).toBe(schedule);
		expect(result.current[1]).toBe(cancel);
	});
});

describe("useConfirmClick", () => {
	it("arms on the first click and confirms on the second", () => {
		const { result } = renderHook(() => useConfirmClick(5000));
		let confirmed = true;
		act(() => {
			confirmed = result.current.confirm();
		});
		expect(confirmed).toBe(false);
		expect(result.current.isConfirming).toBe(true);

		act(() => {
			confirmed = result.current.confirm();
		});
		expect(confirmed).toBe(true);
		expect(result.current.isConfirming).toBe(false);
		expect(result.current.secondsLeft).toBeNull();
	});

	it("counts down once a second and resets after the timeout", () => {
		const { result } = renderHook(() => useConfirmClick(5000));
		act(() => {
			result.current.confirm();
		});
		expect(result.current.secondsLeft).toBe(5);
		act(() => vi.advanceTimersByTime(1000));
		expect(result.current.secondsLeft).toBe(4);
		act(() => vi.advanceTimersByTime(3000));
		expect(result.current.secondsLeft).toBe(1);
		expect(result.current.isConfirming).toBe(true);
		act(() => vi.advanceTimersByTime(1000));
		expect(result.current.isConfirming).toBe(false);
		expect(result.current.secondsLeft).toBeNull();
	});

	it("a click after the reset arms again instead of confirming", () => {
		const { result } = renderHook(() => useConfirmClick(5000));
		act(() => {
			result.current.confirm();
		});
		act(() => vi.advanceTimersByTime(5000));
		let confirmed = true;
		act(() => {
			confirmed = result.current.confirm();
		});
		expect(confirmed).toBe(false);
		expect(result.current.isConfirming).toBe(true);
	});

	it("reset disarms immediately", () => {
		const { result } = renderHook(() => useConfirmClick(5000));
		act(() => {
			result.current.confirm();
		});
		act(() => result.current.reset());
		expect(result.current.isConfirming).toBe(false);
		expect(result.current.secondsLeft).toBeNull();
	});

	it("leaves no timers running after unmount", () => {
		const { result, unmount } = renderHook(() => useConfirmClick(5000));
		act(() => {
			result.current.confirm();
		});
		unmount();
		expect(vi.getTimerCount()).toBe(0);
	});
});
