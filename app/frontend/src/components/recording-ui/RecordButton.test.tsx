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
