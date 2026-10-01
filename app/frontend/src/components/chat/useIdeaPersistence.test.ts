import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import type { ChatMessage } from "@src/types";
import { useIdeaPersistence, type IdeaPersistenceOptions } from "./useIdeaPersistence";

const q1: ChatMessage = { role: "assistant", content: "q1" };
const a1: ChatMessage = { role: "user", content: "a1" };
const q2: ChatMessage = { role: "assistant", content: "q2" };
const a2: ChatMessage = { role: "user", content: "a2" };

function options(overrides: Partial<IdeaPersistenceOptions> = {}): IdeaPersistenceOptions {
	return {
		chatType: "daily_intent",
		parentIdParam: null,
		dailyLogId: null,
		minLength: 2,
		onError: () => {},
		onFatalError: () => {},
		onParentIdea: () => {},
		...overrides
	};
}

function callsTo(command: string) {
	return vi.mocked(invoke).mock.calls.filter(([c]) => c === command);
}

describe("useIdeaPersistence", () => {
	beforeEach(() => {
		window.history.replaceState(null, "", "/chat");
	});
	afterEach(() => {
		vi.useRealTimers();
	});

	it("retries a failed idea creation on the next conversation change", async () => {
		let creates = 0;
		mockInvoke({
			create_idea: () => {
				creates++;
				if (creates === 1) throw new Error("db locked");
				return { id: "idea-1" };
			},
			update_idea: () => ({ id: "idea-1" })
		});
		const onError = vi.fn();
		const { result, rerender } = renderHook(
			({ conversation }) => useIdeaPersistence(conversation, "", options({ onError })),
			{ initialProps: { conversation: [q1, a1] } }
		);
		await waitFor(() => expect(onError).toHaveBeenCalledTimes(1));
		expect(result.current.ideaId).toBeUndefined();

		rerender({ conversation: [q1, a1, q2] });
		await waitFor(() => expect(result.current.ideaId).toBe("idea-1"));
		expect(creates).toBe(2);
	});

	it("creates the idea exactly once under StrictMode, even if the conversation moves on", async () => {
		let resolveCreate!: (v: { id: string }) => void;
		mockInvoke({
			create_idea: () => new Promise((res) => (resolveCreate = res)),
			update_idea: () => ({ id: "idea-1" })
		});
		const { result, rerender } = renderHook(
			({ conversation }) => useIdeaPersistence(conversation, "", options()),
			{ initialProps: { conversation: [q1, a1] }, reactStrictMode: true }
		);
		// a conversation change while the create is in flight must not
		// start a second one
		rerender({ conversation: [q1, a1, q2] });
		rerender({ conversation: [q1, a1, q2, a2] });
		await act(async () => {
			resolveCreate({ id: "idea-1" });
			await Promise.resolve();
		});
		await waitFor(() => expect(result.current.ideaId).toBe("idea-1"));
		expect(callsTo("create_idea")).toHaveLength(1);
	});
});
