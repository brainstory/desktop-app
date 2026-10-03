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
const q3: ChatMessage = { role: "assistant", content: "q3" };

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

	describe("create completion handoff", () => {
		it("saves the newest transcript when creation resolves after unmount", async () => {
			let finish!: (value: { id: string }) => void;
			mockInvoke({
				create_idea: () => new Promise((resolve) => (finish = resolve)),
				update_idea: () => ({ id: "idea-1" })
			});
			const original = [q1, a1];
			const latest = [q1, a1, q2];
			const { rerender, unmount } = renderHook(
				({ conversation }) => useIdeaPersistence(conversation, "", options()),
				{ initialProps: { conversation: original } }
			);
			rerender({ conversation: latest });
			unmount();
			await act(async () => {
				finish({ id: "idea-1" });
			});
			const updates = callsTo("update_idea");
			expect(updates).toHaveLength(1);
			expect(updates[0]![1]).toMatchObject({ id: "idea-1", transcript: latest });
		});

		it("hands the newest transcript to the save queue without waiting for a render or the debounce", async () => {
			vi.useFakeTimers();
			let resolveCreate!: (v: { id: string }) => void;
			mockInvoke({
				create_idea: () => new Promise((res) => (resolveCreate = res)),
				update_idea: () => ({ id: "idea-1" })
			});
			const { rerender } = renderHook(
				({ conversation }) => useIdeaPersistence(conversation, "", options()),
				{ initialProps: { conversation: [q1, a1] } }
			);
			rerender({ conversation: [q1, a1, q2] });
			await act(async () => {
				resolveCreate({ id: "idea-1" });
			});
			await act(() => vi.advanceTimersByTimeAsync(0));
			// saved immediately: no debounce timer has fired yet
			const updates = callsTo("update_idea");
			expect(updates).toHaveLength(1);
			expect((updates[0]![1] as { transcript: ChatMessage[] }).transcript).toEqual([
				q1,
				a1,
				q2
			]);
			await act(() => vi.advanceTimersByTimeAsync(500));
			// the debounced autosave effect must not repeat that write
			expect(callsTo("update_idea")).toHaveLength(1);
		});

		it("retries a rejected create once with the newest conversation and no duplicate writes", async () => {
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
			// the retried create captured the newest conversation itself
			await act(async () => {});
			expect(callsTo("update_idea")).toHaveLength(0);
		});

		it("coalesces rapid edits while the post-create save is in flight", async () => {
			vi.useFakeTimers();
			let resolveCreate!: (v: { id: string }) => void;
			const pending: (() => void)[] = [];
			let inFlight = 0;
			let maxInFlight = 0;
			mockInvoke({
				create_idea: () => new Promise((res) => (resolveCreate = res)),
				update_idea: () =>
					new Promise((res) => {
						inFlight++;
						maxInFlight = Math.max(maxInFlight, inFlight);
						pending.push(() => {
							inFlight--;
							res({ id: "idea-1" });
						});
					})
			});
			const { rerender } = renderHook(
				({ conversation }) => useIdeaPersistence(conversation, "", options()),
				{ initialProps: { conversation: [q1, a1] } }
			);
			rerender({ conversation: [q1, a1, q2] });
			await act(async () => {
				resolveCreate({ id: "idea-1" });
			});
			await act(() => vi.advanceTimersByTimeAsync(0));
			// the newest transcript went straight into the serialized queue
			expect(pending).toHaveLength(1);

			rerender({ conversation: [q1, a1, q2, a2] });
			await act(() => vi.advanceTimersByTimeAsync(500));
			rerender({ conversation: [q1, a1, q2, a2, q3] });
			await act(() => vi.advanceTimersByTimeAsync(500));
			// one update in flight; the newer edits wait behind it
			expect(pending).toHaveLength(1);

			await act(async () => {
				pending.shift()!();
				await vi.advanceTimersByTimeAsync(500);
			});
			expect(maxInFlight).toBe(1);
			const updates = callsTo("update_idea");
			expect(updates).toHaveLength(2);
			expect((updates[1]![1] as { transcript: ChatMessage[] }).transcript).toEqual([
				q1,
				a1,
				q2,
				a2,
				q3
			]);
		});

		it("orders the summary save after the newest-transcript save handed off by the create", async () => {
			vi.useFakeTimers();
			let resolveCreate!: (v: { id: string }) => void;
			let releaseTranscriptSave!: () => void;
			const order: string[] = [];
			mockInvoke({
				create_idea: () => new Promise((res) => (resolveCreate = res)),
				update_idea: (args) => {
					const { result } = args as { result: string };
					if (!result) {
						return new Promise((res) => {
							releaseTranscriptSave = () => {
								order.push("transcript");
								res({ id: "idea-1" });
							};
						});
					}
					order.push("summary");
					return { id: "idea-1" };
				}
			});
			const { result, rerender } = renderHook(
				({ conversation }) => useIdeaPersistence(conversation, "", options()),
				{ initialProps: { conversation: [q1, a1] } }
			);
			rerender({ conversation: [q1, a1, q2] });
			await act(async () => {
				resolveCreate({ id: "idea-1" });
			});
			await act(() => vi.advanceTimersByTimeAsync(0));
			expect(order).toEqual([]);
			let summaryDone = false;
			void result.current.saveResult("idea-1", [q1, a1, q2], "# summary", null).then(() => {
				summaryDone = true;
			});
			await act(() => vi.advanceTimersByTimeAsync(0));
			// the summary waits for the in-flight transcript save
			expect(summaryDone).toBe(false);
			await act(async () => {
				releaseTranscriptSave();
				await vi.advanceTimersByTimeAsync(0);
			});
			expect(order).toEqual(["transcript", "summary"]);
			expect(summaryDone).toBe(true);
		});

		it("a late create completion after teardown does not touch history or a new session", async () => {
			let finish!: (value: { id: string }) => void;
			mockInvoke({
				create_idea: () => new Promise((resolve) => (finish = resolve)),
				update_idea: (args) => ({ id: (args as { id: string }).id })
			});
			const original = [q1, a1];
			const latest = [q1, a1, q2];
			const first = renderHook(
				({ conversation }) => useIdeaPersistence(conversation, "", options()),
				{ initialProps: { conversation: original } }
			);
			first.rerender({ conversation: latest });
			first.unmount();

			// a fresh session owns the same document now
			const second = renderHook(() =>
				useIdeaPersistence([q1], "", options({ initialIdeaId: "idea-b" }))
			);
			await act(async () => {
				finish({ id: "idea-a" });
			});
			// the dead session's newest transcript still reaches its idea row
			const updates = callsTo("update_idea");
			expect(updates).toHaveLength(1);
			expect(updates[0]![1]).toMatchObject({ id: "idea-a", transcript: latest });
			// but the obsolete callback must not rewrite the live URL
			expect(window.location.search).not.toContain("idea-a");
			expect(second.result.current.ideaId).toBe("idea-b");
		});
	});

	describe("autosave", () => {
		it("flushes a pending save on unmount instead of dropping it", async () => {
			vi.useFakeTimers();
			mockInvoke({ update_idea: () => ({ id: "idea-1" }) });
			const { rerender, unmount } = renderHook(
				({ conversation }) =>
					useIdeaPersistence(conversation, "", options({ initialIdeaId: "idea-1" })),
				{ initialProps: { conversation: [q1, a1] } }
			);
			await act(() => vi.advanceTimersByTimeAsync(500));
			vi.mocked(invoke).mockClear();

			rerender({ conversation: [q1, a1, q2] });
			// navigate away before the 400 ms debounce fires
			unmount();
			await act(() => vi.advanceTimersByTimeAsync(0));
			const updates = callsTo("update_idea");
			expect(updates).toHaveLength(1);
			expect((updates[0]![1] as { transcript: ChatMessage[] }).transcript).toEqual([
				q1,
				a1,
				q2
			]);
		});

		it("flushes a pending save on pagehide", async () => {
			vi.useFakeTimers();
			mockInvoke({ update_idea: () => ({ id: "idea-1" }) });
			const { rerender } = renderHook(
				({ conversation }) =>
					useIdeaPersistence(conversation, "", options({ initialIdeaId: "idea-1" })),
				{ initialProps: { conversation: [q1, a1] } }
			);
			await act(() => vi.advanceTimersByTimeAsync(500));
			vi.mocked(invoke).mockClear();

			rerender({ conversation: [q1, a1, q2] });
			window.dispatchEvent(new Event("pagehide"));
			await act(() => vi.advanceTimersByTimeAsync(0));
			expect(callsTo("update_idea")).toHaveLength(1);
		});

		it("serializes writes: one update in flight, the latest conversation wins", async () => {
			vi.useFakeTimers();
			const pending: (() => void)[] = [];
			let inFlight = 0;
			let maxInFlight = 0;
			mockInvoke({
				update_idea: () =>
					new Promise((res) => {
						inFlight++;
						maxInFlight = Math.max(maxInFlight, inFlight);
						pending.push(() => {
							inFlight--;
							res({ id: "idea-1" });
						});
					})
			});
			const { rerender } = renderHook(
				({ conversation }) =>
					useIdeaPersistence(conversation, "", options({ initialIdeaId: "idea-1" })),
				{ initialProps: { conversation: [q1, a1] } }
			);
			await act(() => vi.advanceTimersByTimeAsync(500));
			expect(pending).toHaveLength(1);

			rerender({ conversation: [q1, a1, q2] });
			await act(() => vi.advanceTimersByTimeAsync(500));
			rerender({ conversation: [q1, a1, q2, a2] });
			await act(() => vi.advanceTimersByTimeAsync(500));
			// the first write is still in flight: nothing else may start
			expect(pending).toHaveLength(1);

			await act(async () => {
				pending.shift()!();
				await vi.advanceTimersByTimeAsync(0);
			});
			expect(maxInFlight).toBe(1);
			const updates = callsTo("update_idea");
			// intermediate state coalesced away; the latest one is written
			expect(updates).toHaveLength(2);
			expect((updates[1]![1] as { transcript: ChatMessage[] }).transcript).toEqual([
				q1,
				a1,
				q2,
				a2
			]);
			await act(async () => {
				pending.shift()!();
				await vi.advanceTimersByTimeAsync(500);
			});
			expect(callsTo("update_idea")).toHaveLength(2);
		});

		it("does not re-write the conversation that createIdea just saved", async () => {
			vi.useFakeTimers();
			mockInvoke({
				create_idea: () => ({ id: "idea-1" }),
				update_idea: () => ({ id: "idea-1" })
			});
			const conversation = [q1, a1];
			const { result } = renderHook(() => useIdeaPersistence(conversation, "", options()));
			await act(() => vi.advanceTimersByTimeAsync(0));
			expect(result.current.ideaId).toBe("idea-1");
			await act(() => vi.advanceTimersByTimeAsync(1000));
			expect(callsTo("create_idea")).toHaveLength(1);
			expect(callsTo("update_idea")).toHaveLength(0);
		});

		it("does not write back a conversation marked as already persisted", async () => {
			vi.useFakeTimers();
			mockInvoke({ update_idea: () => ({ id: "idea-1" }) });
			const loaded = [q1, a1, q2, a2];
			const { result, rerender } = renderHook(
				({ conversation }) =>
					useIdeaPersistence(conversation, "", options({ initialIdeaId: "idea-1" })),
				{ initialProps: { conversation: [q1] } }
			);
			act(() => result.current.markPersisted(loaded));
			rerender({ conversation: loaded });
			await act(() => vi.advanceTimersByTimeAsync(1000));
			expect(callsTo("update_idea")).toHaveLength(0);
		});

		it("orders the final result save after an in-flight autosave", async () => {
			vi.useFakeTimers();
			const order: string[] = [];
			let releaseAutosave!: () => void;
			mockInvoke({
				update_idea: (args) => {
					const { result } = args as { result: string };
					if (!result) {
						return new Promise((res) => {
							releaseAutosave = () => {
								order.push("autosave");
								res({ id: "idea-1" });
							};
						});
					}
					order.push("final");
					return { id: "idea-1" };
				}
			});
			const { result } = renderHook(() =>
				useIdeaPersistence([q1, a1], "", options({ initialIdeaId: "idea-1" }))
			);
			await act(() => vi.advanceTimersByTimeAsync(500));
			let finalDone = false;
			void result.current.saveResult("idea-1", [q1, a1], "# summary", null).then(() => {
				finalDone = true;
			});
			await act(() => vi.advanceTimersByTimeAsync(0));
			expect(finalDone).toBe(false);
			await act(async () => {
				releaseAutosave();
				await vi.advanceTimersByTimeAsync(0);
			});
			expect(order).toEqual(["autosave", "final"]);
			expect(finalDone).toBe(true);
		});
	});
});
