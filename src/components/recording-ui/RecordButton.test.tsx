import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { CONVERSATION_STATE } from "@src/const";
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

	describe("transcription", () => {
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
