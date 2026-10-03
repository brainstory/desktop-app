import { afterEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { useVoiceCapture } from "./useVoiceCapture";

function callsTo(command: string) {
	return vi.mocked(invoke).mock.calls.filter(([c]) => c === command);
}

function renderCapture() {
	const callbacks = {
		onWavCaptured: vi.fn(),
		onError: vi.fn(),
		onStopError: vi.fn(),
		onTimeLimit: vi.fn()
	};
	const hook = renderHook(() => {
		const [isRecording, setIsRecording] = useState(false);
		return { isRecording, ...useVoiceCapture({ isRecording, setIsRecording, ...callbacks }) };
	});
	return { ...hook, callbacks };
}

describe("useVoiceCapture", () => {
	afterEach(() => {
		vi.useRealTimers();
	});

	it("starts the Rust-side capture", async () => {
		mockInvoke({ start_voice_capture: () => null });
		const { result } = renderCapture();
		await act(() => result.current.toggleRecording());
		expect(callsTo("start_voice_capture")).toHaveLength(1);
		expect(result.current.isRecording).toBe(true);
		expect(result.current.status).toBe("recording");
	});

	it("releases the mic when unmounted while the start is still in flight", async () => {
		vi.useFakeTimers();
		let started!: (v: unknown) => void;
		mockInvoke({
			start_voice_capture: () => new Promise((res) => (started = res)),
			stop_voice_capture: () => new ArrayBuffer(0)
		});
		const { result, unmount } = renderCapture();
		let toggling!: Promise<void>;
		act(() => {
			toggling = result.current.toggleRecording();
		});
		unmount();
		await act(async () => {
			started(null);
			await toggling;
		});
		// stopped right away, not 4 minutes later
		expect(callsTo("stop_voice_capture")).toHaveLength(1);
		await act(() => vi.advanceTimersByTimeAsync(300_000));
		expect(callsTo("stop_voice_capture")).toHaveLength(1);
	});

	it("reports an error status (not 'stopping mic...') when the stop fails", async () => {
		mockInvoke({
			start_voice_capture: () => null,
			stop_voice_capture: () => {
				throw "device busy";
			}
		});
		const { result, callbacks } = renderCapture();
		await act(() => result.current.toggleRecording());
		await act(() => result.current.toggleRecording());
		expect(callbacks.onStopError).toHaveBeenCalledTimes(1);
		expect(result.current.status).not.toBe("stopping mic...");
		expect(result.current.status).toMatch(/couldn.t stop/i);
		// still recording as far as the UI knows: the stop can be retried
		expect(result.current.isRecording).toBe(true);
	});

	it("retries the stop on unmount after a failed stop", async () => {
		let stops = 0;
		mockInvoke({
			start_voice_capture: () => null,
			stop_voice_capture: () => {
				stops++;
				if (stops === 1) throw "device busy";
				return new ArrayBuffer(0);
			}
		});
		const { result, unmount } = renderCapture();
		await act(() => result.current.toggleRecording());
		await act(() => result.current.toggleRecording());
		unmount();
		expect(callsTo("stop_voice_capture")).toHaveLength(2);
	});

	it("auto-stops at the time limit", async () => {
		vi.useFakeTimers();
		mockInvoke({
			start_voice_capture: () => null,
			stop_voice_capture: () => new ArrayBuffer(1000)
		});
		const { result, callbacks } = renderCapture();
		await act(() => result.current.toggleRecording());
		await act(() => vi.advanceTimersByTimeAsync(239_000));
		expect(callsTo("stop_voice_capture")).toHaveLength(0);
		await act(() => vi.advanceTimersByTimeAsync(1_000));
		expect(callbacks.onTimeLimit).toHaveBeenCalledTimes(1);
		expect(callsTo("stop_voice_capture")).toHaveLength(1);
		expect(callbacks.onWavCaptured).toHaveBeenCalledTimes(1);
		expect(result.current.isRecording).toBe(false);
	});

	it("reports 'no audio captured' for a header-only recording", async () => {
		mockInvoke({
			start_voice_capture: () => null,
			stop_voice_capture: () => new ArrayBuffer(44)
		});
		const { result, callbacks } = renderCapture();
		await act(() => result.current.toggleRecording());
		await act(() => result.current.toggleRecording());
		expect(callbacks.onError).toHaveBeenCalledWith("no audio captured");
		expect(callbacks.onWavCaptured).not.toHaveBeenCalled();
	});

	it("releases the mic on pagehide during a recording (F03)", async () => {
		mockInvoke({
			start_voice_capture: () => null,
			stop_voice_capture: () => new ArrayBuffer(0)
		});
		const { result, callbacks } = renderCapture();
		await act(() => result.current.toggleRecording());
		await act(async () => {
			window.dispatchEvent(new Event("pagehide"));
		});
		expect(callsTo("stop_voice_capture")).toHaveLength(1);
		// abandoned audio is never handed off for transcription
		expect(callbacks.onWavCaptured).not.toHaveBeenCalled();
	});

	it("pagehide teardown is idempotent: a later unmount does not double-stop", async () => {
		mockInvoke({
			start_voice_capture: () => null,
			stop_voice_capture: () => new ArrayBuffer(0)
		});
		const { result, unmount } = renderCapture();
		await act(() => result.current.toggleRecording());
		await act(async () => {
			window.dispatchEvent(new Event("pagehide"));
		});
		expect(callsTo("stop_voice_capture")).toHaveLength(1);
		unmount();
		expect(callsTo("stop_voice_capture")).toHaveLength(1);
	});

	it("releases the mic when the start resolves after pagehide", async () => {
		vi.useFakeTimers();
		let started!: (v: unknown) => void;
		mockInvoke({
			start_voice_capture: () => new Promise((res) => (started = res)),
			stop_voice_capture: () => new ArrayBuffer(0)
		});
		const { result } = renderCapture();
		let toggling!: Promise<void>;
		act(() => {
			toggling = result.current.toggleRecording();
		});
		await act(async () => {
			window.dispatchEvent(new Event("pagehide"));
			started(null);
			await toggling;
		});
		// stopped right away, not armed as a 4-minute recording on a
		// page that is already going away
		expect(callsTo("stop_voice_capture")).toHaveLength(1);
		await act(() => vi.advanceTimersByTimeAsync(300_000));
		expect(callsTo("stop_voice_capture")).toHaveLength(1);
	});

	it("returns to idle when the stop reports 'no audio captured' (device already released)", async () => {
		mockInvoke({
			start_voice_capture: () => null,
			stop_voice_capture: () => {
				throw "no audio captured";
			}
		});
		const { result, callbacks } = renderCapture();
		await act(() => result.current.toggleRecording());
		await act(() => result.current.toggleRecording());
		expect(callbacks.onError).toHaveBeenCalledTimes(1);
		expect(callbacks.onStopError).not.toHaveBeenCalled();
		expect(result.current.isRecording).toBe(false);
		expect(result.current.status).toBe("idle");
	});

	it("returns to idle when the stop reports 'not recording' (already stopped)", async () => {
		mockInvoke({
			start_voice_capture: () => null,
			stop_voice_capture: () => {
				throw "not recording";
			}
		});
		const { result, callbacks } = renderCapture();
		await act(() => result.current.toggleRecording());
		await act(() => result.current.toggleRecording());
		expect(callbacks.onError).toHaveBeenCalledTimes(1);
		expect(callbacks.onStopError).not.toHaveBeenCalled();
		expect(result.current.isRecording).toBe(false);
		expect(result.current.status).toBe("idle");
	});

	it("keeps the permission-denied path for a failed start", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		mockInvoke({
			start_voice_capture: () => {
				throw "not authorized";
			}
		});
		const { result, callbacks } = renderCapture();
		await act(() => result.current.toggleRecording());
		expect(callbacks.onError).toHaveBeenCalledWith("not authorized");
		expect(result.current.isRecording).toBe(false);
		expect(result.current.status).toBe("idle");
		vi.mocked(console.error).mockRestore();
	});
});
