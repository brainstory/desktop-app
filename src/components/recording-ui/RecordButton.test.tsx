import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { CONVERSATION_STATE } from "@src/const";
import { STORAGE_KEYS } from "@src/tauri/commands";
import AudioRecorder from "./AudioRecorder";

/** a WAV-sized buffer (anything over the 44-byte header counts as audio) */
const wav = () => new ArrayBuffer(1000);

function callsTo(command: string) {
	return vi.mocked(invoke).mock.calls.filter(([c]) => c === command);
}

function renderRecorder(conversationState = CONVERSATION_STATE.Idle) {
	const props = {
		conversationState,
		getCoachResponse: vi.fn(async () => "sent" as const),
		onTranscript: vi.fn(),
		setIsTranscribing: vi.fn()
	};
	const view = render(<AudioRecorder {...props} />);
	return { ...view, props };
}

async function recordAndStop() {
	const user = userEvent.setup();
	await user.click(screen.getByRole("button", { name: "Start recording" }));
	await user.click(await screen.findByRole("button", { name: "Stop recording" }));
}

describe("RecordButton", () => {
	afterEach(() => {
		vi.useRealTimers();
		// the input-mode preference must not leak between tests
		localStorage.clear();
	});

	it("announces recording start, stop and transcription to screen readers", async () => {
		let finish!: (v: unknown) => void;
		mockInvoke({
			start_voice_capture: () => null,
			stop_voice_capture: wav,
			transcribe: () => new Promise((res) => (finish = res))
		});
		renderRecorder();
		const user = userEvent.setup();
		await user.click(screen.getByRole("button", { name: "Start recording" }));
		const started = await screen.findByText("Recording started");
		expect(started).toHaveAttribute("aria-live", "polite");
		expect(started).toHaveClass("sr-only");

		await user.click(screen.getByRole("button", { name: "Stop recording" }));
		// the stop is announced, then the transcription that follows it
		expect(await screen.findByText("Transcribing…")).toHaveAttribute("aria-live", "polite");
		await act(async () => {
			finish({ transcript: "hello" });
			await Promise.resolve();
		});
		expect(screen.queryByText("Transcribing…")).not.toBeInTheDocument();
	});

	it("remembers the typed-input preference across mounts", async () => {
		mockInvoke({
			start_voice_capture: () => null,
			stop_voice_capture: wav
		});
		// a previous session chose typing
		localStorage.setItem(STORAGE_KEYS.inputMode, "text");
		const props = {
			conversationState: CONVERSATION_STATE.Idle,
			getCoachResponse: vi.fn(async () => "sent" as const),
			onTranscript: vi.fn(),
			setIsTranscribing: vi.fn()
		};
		const { unmount } = render(<AudioRecorder {...props} />);
		expect(screen.getByLabelText("Type your response")).toBeInTheDocument();

		// switch back to the microphone: the choice persists
		await userEvent.click(screen.getByText("Use the microphone"));
		expect(localStorage.getItem(STORAGE_KEYS.inputMode)).toBe("voice");
		unmount();
		cleanup();

		// a fresh mount starts on the microphone again
		render(<AudioRecorder {...props} />);
		expect(screen.getByRole("button", { name: "Start recording" })).toBeInTheDocument();

		// and a blocked mic defaults to typing even without a stored choice
		localStorage.removeItem(STORAGE_KEYS.inputMode);
		cleanup();
		render(<AudioRecorder {...props} micBlockedReason="Speech model needed" />);
		expect(screen.getByLabelText("Type your response")).toBeInTheDocument();

		// never leak the preference into the other tests
		localStorage.removeItem(STORAGE_KEYS.inputMode);
	});

	it("announces a stop that is not followed by a transcription", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		mockInvoke({
			start_voice_capture: () => null,
			stop_voice_capture: () => new ArrayBuffer(44)
		});
		renderRecorder();
		await recordAndStop();
		expect(await screen.findByText("Recording stopped")).toHaveAttribute("aria-live", "polite");
		vi.mocked(console.error).mockRestore();
	});

	describe("keyboard shortcuts", () => {
		it("Space toggles the mic when no field has focus", async () => {
			mockInvoke({
				start_voice_capture: () => null,
				stop_voice_capture: wav
			});
			renderRecorder();
			const user = userEvent.setup();
			await user.keyboard(" ");
			expect(
				vi.mocked(invoke).mock.calls.filter(([c]) => c === "start_voice_capture")
			).toHaveLength(1);
			await user.keyboard(" ");
			expect(
				vi.mocked(invoke).mock.calls.filter(([c]) => c === "stop_voice_capture")
			).toHaveLength(1);
		});

		it("typing a space in the composer does not toggle the mic", async () => {
			mockInvoke({
				start_voice_capture: () => null,
				stop_voice_capture: wav
			});
			renderRecorder();
			const user = userEvent.setup();
			await user.click(screen.getByText("Type instead"));
			const box = screen.getByLabelText("Type your response");
			await user.click(box);
			await user.keyboard("hi there ");
			expect(
				vi.mocked(invoke).mock.calls.filter(([c]) => c === "start_voice_capture")
			).toHaveLength(0);
			expect(box).toHaveValue("hi there ");
		});

		it("Escape cancels a pending transcription", async () => {
			let finish!: (v: unknown) => void;
			mockInvoke({
				start_voice_capture: () => null,
				stop_voice_capture: wav,
				transcribe: () => new Promise((res) => (finish = res)),
				cancel_transcription: () => null
			});
			renderRecorder();
			const user = userEvent.setup();
			await user.click(screen.getByRole("button", { name: "Start recording" }));
			await user.click(screen.getByRole("button", { name: "Stop recording" }));
			expect(await screen.findByText("Transcribing…")).toBeInTheDocument();

			await user.keyboard("{Escape}");
			expect(callsTo("cancel_transcription")).toHaveLength(1);
			await act(async () => {
				finish({ transcript: "late" });
				await Promise.resolve();
			});
		});
	});

	describe("transcription", () => {
		it("cancels a pending transcription without an error banner", async () => {
			let fail!: (e: unknown) => void;
			mockInvoke({
				start_voice_capture: () => null,
				stop_voice_capture: wav,
				transcribe: () => new Promise((_res, rej) => (fail = rej)),
				cancel_transcription: () => null
			});
			const { props } = renderRecorder();
			await recordAndStop();
			expect(await screen.findByText("Transcribing…")).toBeInTheDocument();

			const user = userEvent.setup();
			await user.click(screen.getByRole("button", { name: "Cancel transcription" }));
			expect(callsTo("cancel_transcription")).toHaveLength(1);
			// the recorder is released immediately
			await waitFor(() => expect(props.setIsTranscribing).toHaveBeenLastCalledWith(false));
			expect(screen.queryByText("Transcribing…")).not.toBeInTheDocument();

			// the pending invoke settles with the backend's cancel error:
			// no banner, no retry
			await act(async () => {
				fail("transcription cancelled");
				await Promise.resolve();
			});
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			expect(callsTo("transcribe")).toHaveLength(1);
		});

		it("does not retry a permanent transcription error", async () => {
			vi.useFakeTimers({ shouldAdvanceTime: true });
			vi.spyOn(console, "error").mockImplementation(() => {});
			mockInvoke({
				start_voice_capture: () => null,
				stop_voice_capture: wav,
				transcribe: () => {
					throw "no audio received";
				}
			});
			renderRecorder();
			await recordAndStop();
			expect(await screen.findByRole("alert")).toHaveTextContent("No audio was captured.");
			await act(() => vi.advanceTimersByTimeAsync(2000));
			expect(callsTo("transcribe")).toHaveLength(1);
			vi.mocked(console.error).mockRestore();
		});

		it("unmounting cancels a pending transcription retry", async () => {
			vi.useFakeTimers({ shouldAdvanceTime: true });
			mockInvoke({
				start_voice_capture: () => null,
				stop_voice_capture: wav,
				transcribe: () => {
					throw "connection reset";
				}
			});
			const { unmount } = renderRecorder();
			await recordAndStop();
			await vi.waitFor(() => expect(callsTo("transcribe")).toHaveLength(1));
			unmount();
			await vi.advanceTimersByTimeAsync(2000);
			expect(callsTo("transcribe")).toHaveLength(1);
		});

		it("says nothing was heard instead of sending an empty message", async () => {
			mockInvoke({
				start_voice_capture: () => null,
				stop_voice_capture: wav,
				transcribe: () => ({ transcript: "  " })
			});
			const { props } = renderRecorder();
			await recordAndStop();
			expect(await screen.findByRole("alert")).toHaveTextContent("No speech was heard.");
			expect(props.onTranscript).not.toHaveBeenCalled();
			expect(props.setIsTranscribing).toHaveBeenLastCalledWith(false);
		});

		it("restarts the recording timer at 0:00 after a failed transcription", async () => {
			vi.useFakeTimers({ shouldAdvanceTime: true });
			vi.spyOn(console, "error").mockImplementation(() => {});
			let calls = 0;
			mockInvoke({
				start_voice_capture: () => null,
				stop_voice_capture: wav,
				transcribe: () => {
					calls += 1;
					if (calls === 1) throw "no audio received";
					return { transcript: "hello" };
				}
			});
			renderRecorder();
			// first recording runs long enough that a stale timer would
			// trigger the "almost reached" warning on the next one
			const user = userEvent.setup();
			await user.click(screen.getByRole("button", { name: "Start recording" }));
			await act(() => vi.advanceTimersByTimeAsync(215_000));
			expect(screen.getByText("recording · 3:35")).toBeInTheDocument();
			await user.click(screen.getByRole("button", { name: "Stop recording" }));
			expect(await screen.findByRole("alert")).toBeInTheDocument();

			// second recording: the readout starts at zero again...
			await user.click(screen.getByRole("button", { name: "Start recording" }));
			expect(screen.getByText("recording · 0:00")).toBeInTheDocument();
			// ...and the 4-minute warning is not carried over from the
			// failed attempt
			expect(screen.queryByText(/almost reached/)).not.toBeInTheDocument();
			vi.mocked(console.error).mockRestore();
		});

		it("ignores a transcript that arrives after the recorder unmounted", async () => {
			let finish!: (v: unknown) => void;
			mockInvoke({
				start_voice_capture: () => null,
				stop_voice_capture: wav,
				transcribe: () => new Promise((res) => (finish = res))
			});
			const { props, unmount } = renderRecorder();
			await recordAndStop();
			expect(callsTo("transcribe")).toHaveLength(1);
			unmount();
			await act(async () => {
				finish({ transcript: "too late" });
				await Promise.resolve();
			});
			expect(props.onTranscript).not.toHaveBeenCalled();
		});
	});
});
