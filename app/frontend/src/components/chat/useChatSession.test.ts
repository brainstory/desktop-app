import { afterEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { CONVERSATION_STATE } from "@src/const";
import type { ChatMessage } from "@src/types";
import {
	chatSessionReducer,
	initialChatSessionState,
	useChatSession,
	type ChatSessionEvent,
	type ChatSessionOptions
} from "./useChatSession";

const q1: ChatMessage = { role: "assistant", content: "q1" };
const a1: ChatMessage = { role: "user", content: "a1" };

function renderSession(initial: ChatMessage[], overrides: Partial<ChatSessionOptions> = {}) {
	const opts: ChatSessionOptions = {
		chatType: "original",
		ideaId: "idea-1",
		onError: vi.fn(),
		conversationEndCallbacks: () => {},
		setSaveState: () => {},
		saveResult: vi.fn(async () => {}),
		setResult: vi.fn(),
		...overrides
	};
	const hook = renderHook(() => {
		const [conversation, setConversation] = useState(initial);
		return { conversation, ...useChatSession(conversation, setConversation, opts) };
	});
	return { ...hook, opts };
}

function callsTo(command: string) {
	return vi.mocked(invoke).mock.calls.filter(([c]) => c === command);
}

/** the Channel the streaming command was handed (see test/setup.ts) */
function streamChannel(): { onmessage: (event: unknown) => void } {
	const args = callsTo("generate_streaming_response")[0]![1] as {
		onEvent: { onmessage: (event: unknown) => void };
	};
	return args.onEvent;
}

describe("useChatSession", () => {
	afterEach(() => {
		vi.useRealTimers();
	});

	describe("coach response", () => {
		it("adds the answer and reports 'sent'", async () => {
			mockInvoke({ generate_response: () => ({ response: "q2" }) });
			const { result } = renderSession([q1, a1]);
			let outcome: string | undefined;
			await act(async () => {
				outcome = await result.current.handleGetResponse();
			});
			expect(outcome).toBe("sent");
			expect(result.current.conversation).toEqual([
				q1,
				a1,
				{ role: "assistant", content: "q2" }
			]);
			expect(result.current.conversationState).toBe(CONVERSATION_STATE.Idle);
		});

		it("removes the flagged user message on a moderation error", async () => {
			mockInvoke({
				generate_response: () => {
					throw "HttpError 469: Inappropriate input";
				}
			});
			const { result, opts } = renderSession([q1, a1]);
			let outcome: string | undefined;
			await act(async () => {
				outcome = await result.current.handleGetResponse();
			});
			expect(outcome).toBe("flagged");
			expect(result.current.conversation).toEqual([q1]);
			expect(result.current.isUserResendRequired).toBe(true);
			expect(result.current.inappropriateUserTranscript).toBe("a1");
			expect(opts.onError).not.toHaveBeenCalled();
			expect(callsTo("generate_response")).toHaveLength(1);
		});

		it("reports other AI errors through onError and keeps the message", async () => {
			vi.useFakeTimers();
			mockInvoke({
				generate_response: () => {
					throw "endpoint down";
				}
			});
			const { result, opts } = renderSession([q1, a1]);
			let outcome: string | undefined;
			await act(async () => {
				const pending = result.current.handleGetResponse().then((o) => (outcome = o));
				await vi.advanceTimersByTimeAsync(1000);
				await pending;
			});
			expect(outcome).toBe("failed");
			expect(opts.onError).toHaveBeenCalledWith("endpoint down");
			expect(result.current.conversation).toEqual([q1, a1]);
		});

		it("unmounting cancels a pending retry", async () => {
			vi.useFakeTimers();
			mockInvoke({
				generate_response: () => {
					throw "endpoint down";
				}
			});
			const { result, opts, unmount } = renderSession([q1, a1]);
			let outcome: string | undefined;
			let pending!: Promise<unknown>;
			await act(async () => {
				pending = result.current.handleGetResponse().then((o) => (outcome = o));
				await vi.advanceTimersByTimeAsync(100);
			});
			expect(callsTo("generate_response")).toHaveLength(1);
			unmount();
			await vi.advanceTimersByTimeAsync(2000);
			await pending;
			expect(callsTo("generate_response")).toHaveLength(1);
			expect(outcome).toBe("cancelled");
			expect(opts.onError).not.toHaveBeenCalled();
		});
	});

	describe("cancelling a coach response", () => {
		it("does not retry a cancelled generation or show an error", async () => {
			vi.useFakeTimers();
			mockInvoke({
				generate_response: () => {
					throw "generation cancelled";
				}
			});
			const { result, opts } = renderSession([q1, a1]);
			await act(async () => {
				void result.current.handleGetResponse();
				await vi.advanceTimersByTimeAsync(2000);
			});
			expect(callsTo("generate_response")).toHaveLength(1);
			expect(opts.onError).not.toHaveBeenCalled();
			expect(result.current.conversationState).toBe(CONVERSATION_STATE.Idle);
			// the user's message stays so the chat can continue
			expect(result.current.conversation).toEqual([q1, a1]);
		});
	});

	describe("final summary", () => {
		it("saves the result, then marks it complete and ready", async () => {
			let finish!: (v: unknown) => void;
			mockInvoke({
				generate_streaming_response: () => new Promise((res) => (finish = res))
			});
			const saveResult = vi.fn(async () => {});
			const { result, opts } = renderSession([q1, a1], { saveResult });
			act(() => result.current.handleGetResult());
			act(() => streamChannel().onmessage({ type: "chunk", content: "# Sum" }));
			expect(result.current.resultComplete).toBe(false);
			expect(result.current.readyToSave).toBe(false);

			await act(async () => {
				finish({ response: "# Summary", structured_result: null });
				await Promise.resolve();
			});
			expect(opts.setResult).toHaveBeenLastCalledWith("# Summary");
			expect(saveResult).toHaveBeenCalledWith("idea-1", [q1, a1], "# Summary", null);
			expect(result.current.resultComplete).toBe(true);
			expect(result.current.readyToSave).toBe(true);
		});
	});

	describe("cancelling the final summary", () => {
		it("returns to the chat without an error banner", async () => {
			let rejectStream!: (e: unknown) => void;
			mockInvoke({
				generate_streaming_response: () =>
					new Promise((_res, rej) => {
						rejectStream = rej;
					})
			});
			const { result, opts } = renderSession([q1, a1]);
			act(() => result.current.handleGetResult());
			expect(result.current.conversationState).toBe(CONVERSATION_STATE.FinishWithResult);
			act(() => streamChannel().onmessage({ type: "chunk", content: "# partial" }));
			expect(opts.setResult).toHaveBeenLastCalledWith("# partial");

			await act(async () => {
				rejectStream("generation cancelled");
				await Promise.resolve();
			});
			expect(opts.onError).not.toHaveBeenCalled();
			// the partial summary is cleared so the chat view comes back
			expect(opts.setResult).toHaveBeenLastCalledWith("");
			expect(result.current.conversationState).toBe(CONVERSATION_STATE.Idle);
		});

		it("clears a partial summary on failure so the error banner is visible", async () => {
			let rejectStream!: (e: unknown) => void;
			mockInvoke({
				generate_streaming_response: () =>
					new Promise((_res, rej) => {
						rejectStream = rej;
					})
			});
			const { result, opts } = renderSession([q1, a1]);
			act(() => result.current.handleGetResult());
			act(() => streamChannel().onmessage({ type: "chunk", content: "# partial" }));
			await act(async () => {
				rejectStream("endpoint did not return an SSE stream");
				await Promise.resolve();
			});
			expect(opts.onError).toHaveBeenCalledTimes(1);
			expect(opts.setResult).toHaveBeenLastCalledWith("");
			expect(result.current.conversationState).toBe(CONVERSATION_STATE.Idle);
		});
	});

	it("keeps handleGetResponse stable across re-renders of the same conversation", () => {
		const { result, rerender } = renderSession([q1, a1]);
		const first = result.current.handleGetResponse;
		rerender();
		expect(result.current.handleGetResponse).toBe(first);
	});

	describe("chatSessionReducer", () => {
		const run = (...events: ChatSessionEvent[]) =>
			events.reduce(chatSessionReducer, initialChatSessionState);

		it("walks a voice turn: transcribe -> ready -> waiting -> idle", () => {
			expect(run({ type: "transcriptionStarted" }).conversationState).toBe(
				CONVERSATION_STATE.TranscribingUser
			);
			expect(
				run({ type: "transcriptionStarted" }, { type: "userMessageReady" })
					.conversationState
			).toBe(CONVERSATION_STATE.ReadyToSendUserTranscript);
			expect(run({ type: "coachRequested" }).conversationState).toBe(
				CONVERSATION_STATE.WaitingForCoach
			);
			expect(
				run({ type: "coachRequested" }, { type: "coachAnswered" }).conversationState
			).toBe(CONVERSATION_STATE.Idle);
		});

		it("tracks the moderation resend flow", () => {
			const flagged = run(
				{ type: "coachRequested" },
				{ type: "coachFlagged", transcript: "x" }
			);
			expect(flagged).toMatchObject({
				conversationState: CONVERSATION_STATE.Idle,
				isUserResendRequired: true,
				inappropriateUserTranscript: "x"
			});
			expect(chatSessionReducer(flagged, { type: "coachAnswered" })).toMatchObject({
				isUserResendRequired: false,
				inappropriateUserTranscript: null
			});
		});

		it("restores a draft to the right next step", () => {
			expect(run({ type: "draftRestored", lastRole: "user" }).conversationState).toBe(
				CONVERSATION_STATE.ReadyToSendUserTranscript
			);
			expect(run({ type: "draftRestored", lastRole: "assistant" }).conversationState).toBe(
				CONVERSATION_STATE.Idle
			);
		});

		it("tracks the summary: requested -> streamed -> saved, or back to idle", () => {
			expect(
				run(
					{ type: "resultRequested" },
					{ type: "resultStreamed" },
					{ type: "resultSaved" }
				)
			).toMatchObject({
				conversationState: CONVERSATION_STATE.FinishWithResult,
				resultComplete: true,
				readyToSave: true
			});
			expect(
				run(
					{ type: "resultRequested" },
					{ type: "resultStreamed" },
					{ type: "resultFailed" }
				)
			).toMatchObject({ conversationState: CONVERSATION_STATE.Idle, resultComplete: false });
			expect(
				run(
					{ type: "resultRequested" },
					{ type: "resultStreamed" },
					{ type: "resultSaveFailed" }
				).conversationState
			).toBe(CONVERSATION_STATE.Idle);
		});
	});
});
