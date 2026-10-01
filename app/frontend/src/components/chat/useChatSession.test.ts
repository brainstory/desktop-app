import { afterEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { CONVERSATION_STATE } from "@src/const";
import type { ChatMessage } from "@src/types";
import { useChatSession, type ChatSessionOptions } from "./useChatSession";

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
});
