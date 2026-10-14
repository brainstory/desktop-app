import { afterEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { CONVERSATION_STATE, ASK_A_DIFFERENT_QUESTION } from "@src/const";
import type { ChatMessage } from "@src/types";
import {
	chatSessionReducer,
	initialChatSessionState,
	useChatSession,
	type ChatSessionEvent,
	type ChatSessionOptions,
	type CoachResponseOutcome
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
		it("streams the reply into view and appends exactly one message at the end", async () => {
			let finish!: (v: unknown) => void;
			mockInvoke({
				generate_streaming_response: () => new Promise((res) => (finish = res))
			});
			const { result } = renderSession([q1, a1]);
			let settled: Promise<CoachResponseOutcome> | undefined;
			act(() => {
				settled = result.current.handleGetResponse();
			});
			const channel = streamChannel();
			await act(async () => {
				channel.onmessage({ type: "chunk", content: "one " });
				channel.onmessage({ type: "chunk", content: "two " });
				channel.onmessage({ type: "chunk", content: "three" });
				await Promise.resolve();
			});
			// the partial reply is visible while the coach is still talking
			expect(result.current.streamingReply).toBe("one two three");
			expect(result.current.conversationState).toBe(CONVERSATION_STATE.WaitingForCoach);
			expect(result.current.conversation).toHaveLength(2);

			let outcome: CoachResponseOutcome | undefined;
			await act(async () => {
				finish({ response: "one two three and done" });
				outcome = await settled;
			});
			// ...and the conversation gains exactly one assistant message
			expect(outcome).toBe("sent");
			expect(result.current.streamingReply).toBeNull();
			expect(result.current.conversation).toHaveLength(3);
			expect(result.current.conversation[2]).toEqual({
				role: "assistant",
				content: "one two three and done"
			});
		});

		it("adds the answer and reports 'sent'", async () => {
			mockInvoke({ generate_streaming_response: () => ({ response: "q2" }) });
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
				generate_streaming_response: () => {
					throw "moderation: the AI provider flagged this message";
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
			expect(callsTo("generate_streaming_response")).toHaveLength(1);
		});

		it("reports other AI errors through onError and keeps the message", async () => {
			vi.useFakeTimers();
			mockInvoke({
				generate_streaming_response: () => {
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
				generate_streaming_response: () => {
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
			expect(callsTo("generate_streaming_response")).toHaveLength(1);
			unmount();
			await vi.advanceTimersByTimeAsync(2000);
			await pending;
			expect(callsTo("generate_streaming_response")).toHaveLength(1);
			expect(outcome).toBe("cancelled");
			expect(opts.onError).not.toHaveBeenCalled();
		});
	});

	describe("cancelling a coach response", () => {
		it("does not retry a cancelled generation or show an error", async () => {
			vi.useFakeTimers();
			mockInvoke({
				generate_streaming_response: () => {
					throw "generation cancelled";
				}
			});
			const { result, opts } = renderSession([q1, a1]);
			await act(async () => {
				void result.current.handleGetResponse();
				await vi.advanceTimersByTimeAsync(2000);
			});
			expect(callsTo("generate_streaming_response")).toHaveLength(1);
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

		it("records when the summary was generated from a truncated transcript", async () => {
			let finish!: (v: unknown) => void;
			mockInvoke({
				generate_streaming_response: () => new Promise((res) => (finish = res))
			});
			const { result } = renderSession([q1, a1]);
			act(() => result.current.handleGetResult());
			expect(result.current.resultTruncated).toBe(false);

			await act(async () => {
				finish({
					response: "# Summary",
					structured_result: null,
					truncated: true,
					dropped_turns: 14
				});
				await Promise.resolve();
			});
			expect(result.current.resultTruncated).toBe(true);
			expect(result.current.resultDroppedTurns).toBe(14);

			// a fresh generation clears the old flag
			mockInvoke({
				generate_streaming_response: () => new Promise((res) => (finish = res))
			});
			act(() => result.current.handleGetResult());
			expect(result.current.resultTruncated).toBe(false);
			expect(result.current.resultDroppedTurns).toBe(0);
		});

		it("keeps the summary after a failed save and lets the user retry it", async () => {
			let finish!: (v: unknown) => void;
			mockInvoke({
				generate_streaming_response: () => new Promise((res) => (finish = res))
			});
			const saveResult = vi
				.fn<ChatSessionOptions["saveResult"]>()
				.mockRejectedValueOnce("disk full")
				.mockResolvedValueOnce(undefined);
			const { result, opts } = renderSession([q1, a1], { saveResult });
			act(() => result.current.handleGetResult());
			await act(async () => {
				finish({ response: "# Summary", structured_result: null });
				await Promise.resolve();
			});
			expect(result.current.resultSaveError).toBe("disk full");
			expect(result.current.readyToSave).toBe(false);
			// the summary stays set: the result view shows the error there
			expect(opts.setResult).toHaveBeenLastCalledWith("# Summary");
			expect(opts.setResult).not.toHaveBeenCalledWith("");

			await act(async () => {
				result.current.retrySaveResult();
				await Promise.resolve();
			});
			expect(saveResult).toHaveBeenCalledTimes(2);
			expect(saveResult).toHaveBeenLastCalledWith("idea-1", [q1, a1], "# Summary", null);
			expect(result.current.resultSaveError).toBeNull();
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

	describe("revising the last answer", () => {
		const q2: ChatMessage = { role: "assistant", content: "q2" };
		const a2: ChatMessage = { role: "user", content: "a2" };
		const q3: ChatMessage = { role: "assistant", content: "q3" };
		/** The real session reaches Idle through its events (or a draft
		 * restore); the hook test starts from a directly-provided
		 * conversation, so it takes the same door. */
		const settle = (result: { current: { dispatch: (e: ChatSessionEvent) => void } }) =>
			act(() => result.current.dispatch({ type: "draftRestored", lastRole: "assistant" }));

		it("edits the answer, retires the reply to it, and asks again", async () => {
			mockInvoke({ generate_streaming_response: () => ({ response: "q4" }) });
			// a settled exchange: q3 is the coach's reply to a2
			const { result } = renderSession([q1, a1, q2, a2, q3]);
			settle(result);
			expect(result.current.canReviseLastAnswer).toBe(true);
			await act(async () => {
				await result.current.editLastUserMessage("a2 fixed");
			});
			expect(result.current.conversation).toEqual([
				q1,
				a1,
				q2,
				{ role: "user", content: "a2 fixed" },
				{ role: "assistant", content: "q4" }
			]);
			expect(result.current.conversationState).toBe(CONVERSATION_STATE.Idle);
		});

		it("does not offer revision while the coach is generating", async () => {
			mockInvoke({ generate_response: () => new Promise(() => {}) });
			const { result } = renderSession([q1, a1]);
			act(() => {
				void result.current.handleGetResponse();
			});
			expect(result.current.conversationState).toBe(CONVERSATION_STATE.WaitingForCoach);
			expect(result.current.canReviseLastAnswer).toBe(false);
			// and a direct call is a no-op, not a second generation
			await act(async () => {
				await result.current.editLastUserMessage("too late");
			});
			expect(callsTo("generate_streaming_response")).toHaveLength(1);
		});

		it("removes the last answer and its reply, leaving the question open", () => {
			const { result } = renderSession([q1, a1, q2, a2, q3]);
			settle(result);
			act(() => result.current.removeLastUserTurn());
			expect(result.current.conversation).toEqual([q1, a1, q2]);
			expect(result.current.conversationState).toBe(CONVERSATION_STATE.Idle);
			// the previous exchange is now the last one: it becomes the
			// revisable one (the question is on screen again)
			expect(result.current.canReviseLastAnswer).toBe(true);
		});

		it("never offers revision of the meta 'ask a different question' turn", () => {
			const { result } = renderSession([
				q1,
				{ role: "user", content: ASK_A_DIFFERENT_QUESTION },
				q2
			]);
			settle(result);
			expect(result.current.canReviseLastAnswer).toBe(false);
		});
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
