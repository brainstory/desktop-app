import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanStores } from "nanostores";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { mockInvoke } from "@src/test/mock-tauri";
import { $aiStatus, initAiStatus, llmAvailability, llmBusy, type AiStatus } from "./aiStatusStore";

type Handler = (event: { payload: unknown }) => void;

/** capture the status-event handlers the store registers */
function captureListeners() {
	const handlers: Record<string, Handler> = {};
	const order: string[] = [];
	vi.mocked(listen).mockImplementation(((name: string, handler: Handler) => {
		order.push(`listen:${name}`);
		handlers[name] = handler;
		return Promise.resolve(() => {});
	}) as unknown as typeof listen);
	return { handlers, order };
}

/** capture handlers plus every listen/unlisten call; `script` queues a
 * rejected listen per event name (one Error per failing call), all
 * other and later calls acquire a listener that records its release */
function captureListenerCalls(script: Record<string, Error[]> = {}) {
	const handlers: Record<string, Handler> = {};
	const calls: string[] = [];
	const unlistened: string[] = [];
	vi.mocked(listen).mockImplementation(((name: string, handler: Handler) => {
		calls.push(`listen:${name}`);
		handlers[name] = handler;
		const failure = script[name]?.shift();
		if (failure) return Promise.reject(failure);
		return Promise.resolve(() => {
			unlistened.push(name);
		});
	}) as unknown as typeof listen);
	return { handlers, calls, unlistened };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function runtimeStatusCalls() {
	return vi.mocked(invoke).mock.calls.filter(([c]) => c === "get_runtime_status");
}

describe("aiStatusStore", () => {
	afterEach(() => {
		// runs the onMount teardown: the next test starts from scratch
		cleanStores($aiStatus);
		vi.mocked(listen).mockReset();
	});

	it("starts unknown instead of claiming the model is missing", () => {
		captureListeners();
		// the read never answers: what do consumers see meanwhile?
		mockInvoke({ get_runtime_status: () => new Promise(() => {}) });
		let seen: AiStatus | undefined;
		const unsubscribe = $aiStatus.subscribe((value) => (seen = value));
		expect(seen?.loaded).toBe(false);
		expect(llmAvailability(seen!)).toBe("unknown");
		expect(llmBusy(seen!)).toBe(false);
		unsubscribe();
	});

	it("seeds itself lazily when the first consumer subscribes", async () => {
		captureListeners();
		mockInvoke({
			get_runtime_status: () => ({ llm: { state: "ready" }, stt: { state: "missing" } })
		});
		expect(runtimeStatusCalls()).toHaveLength(0);
		const unsubscribe = $aiStatus.subscribe(() => {});
		await flush();
		expect(runtimeStatusCalls()).toHaveLength(1);
		expect($aiStatus.get()).toEqual({
			llm: { state: "ready" },
			stt: { state: "missing" },
			loaded: true
		});
		unsubscribe();
	});

	it("is idempotent: many subscribers and init calls share one start", async () => {
		captureListeners();
		mockInvoke({
			get_runtime_status: () => ({ llm: { state: "ready" }, stt: { state: "ready" } })
		});
		const a = $aiStatus.subscribe(() => {});
		const b = $aiStatus.listen(() => {});
		void initAiStatus();
		void initAiStatus();
		await flush();
		expect(runtimeStatusCalls()).toHaveLength(1);
		expect(vi.mocked(listen)).toHaveBeenCalledTimes(2);
		a();
		b();
	});

	it("attaches the event listeners before reading the initial status", async () => {
		const { order } = captureListeners();
		mockInvoke({
			get_runtime_status: () => {
				order.push("read");
				return { llm: { state: "loading" }, stt: { state: "ready" } };
			}
		});
		await initAiStatus();
		expect(order).toEqual(["listen:llm-status", "listen:stt-status", "read"]);
	});

	it("keeps an event that arrives while the initial read is in flight", async () => {
		const { handlers } = captureListeners();
		let answer!: (v: unknown) => void;
		mockInvoke({ get_runtime_status: () => new Promise((res) => (answer = res)) });
		const started = initAiStatus();
		await flush();
		// the model finished loading after the read was issued...
		handlers["llm-status"]!({ payload: { state: "ready" } });
		// ...and the (older) read answer still says loading
		answer({ llm: { state: "loading" }, stt: { state: "ready" } });
		await started;
		expect($aiStatus.get().llm).toEqual({ state: "ready" });
		expect($aiStatus.get().stt).toEqual({ state: "ready" });
		expect($aiStatus.get().loaded).toBe(true);
	});

	it("applies live status events", async () => {
		const { handlers } = captureListeners();
		mockInvoke({
			get_runtime_status: () => ({ llm: { state: "loading" }, stt: { state: "ready" } })
		});
		await initAiStatus();
		handlers["llm-status"]!({ payload: { state: "error", error: "bad gguf" } });
		expect(llmAvailability($aiStatus.get())).toBe("error");
	});

	it("stays unknown when the initial read fails", async () => {
		captureListeners();
		vi.spyOn(console, "error").mockImplementation(() => {});
		mockInvoke({
			get_runtime_status: () => {
				throw new Error("ipc down");
			}
		});
		await initAiStatus();
		expect(llmAvailability($aiStatus.get())).toBe("unknown");
		vi.mocked(console.error).mockRestore();
	});

	describe("recovers from transient startup failures", () => {
		beforeEach(() => {
			vi.useFakeTimers();
			vi.spyOn(console, "error").mockImplementation(() => {});
		});

		afterEach(() => {
			// run the onMount teardown while the retry timers it must
			// cancel are still faked, then restore the real clock
			cleanStores($aiStatus);
			vi.useRealTimers();
			vi.mocked(console.error).mockRestore();
		});

		const settle = () => vi.advanceTimersByTimeAsync(0);

		it("resolves, releases the acquired sibling listener and retries when the second listen rejects", async () => {
			const { calls, unlistened } = captureListenerCalls({
				"stt-status": [new Error("ipc down")]
			});
			mockInvoke({
				get_runtime_status: () => ({ llm: { state: "ready" }, stt: { state: "missing" } })
			});
			const unsubscribe = $aiStatus.subscribe(() => {});
			// a failed start must not reject: onMount fire-and-forgets it
			await expect(initAiStatus()).resolves.toBeUndefined();
			// the llm listener acquired before stt rejected is released
			expect(unlistened).toEqual(["llm-status"]);
			// the bounded retry re-acquires both listeners and reads
			await vi.advanceTimersByTimeAsync(250);
			expect(calls).toEqual([
				"listen:llm-status",
				"listen:stt-status",
				"listen:llm-status",
				"listen:stt-status"
			]);
			expect($aiStatus.get()).toEqual({
				llm: { state: "ready" },
				stt: { state: "missing" },
				loaded: true
			});
			unsubscribe();
		});

		it("retries when the first listen rejects without leaking or duplicating listeners", async () => {
			const { calls, unlistened } = captureListenerCalls({
				"llm-status": [new Error("ipc down")]
			});
			mockInvoke({
				get_runtime_status: () => ({ llm: { state: "loading" }, stt: { state: "ready" } })
			});
			const unsubscribe = $aiStatus.subscribe(() => {});
			await expect(initAiStatus()).resolves.toBeUndefined();
			await vi.advanceTimersByTimeAsync(250);
			// the failed attempt acquired nothing (no leak) and the retry
			// acquires each listener exactly once
			expect(calls).toEqual(["listen:llm-status", "listen:llm-status", "listen:stt-status"]);
			expect(unlistened).toEqual([]);
			expect($aiStatus.get().loaded).toBe(true);
			unsubscribe();
		});

		it("reaches loaded=true when the initial read fails but a retry succeeds", async () => {
			captureListenerCalls();
			let reads = 0;
			mockInvoke({
				get_runtime_status: () => {
					reads++;
					if (reads === 1) throw new Error("ipc down");
					return { llm: { state: "ready" }, stt: { state: "ready" } };
				}
			});
			const unsubscribe = $aiStatus.subscribe(() => {});
			await settle();
			// failed read: still unknown, never a wrong "missing" claim
			expect($aiStatus.get().loaded).toBe(false);
			expect(llmAvailability($aiStatus.get())).toBe("unknown");
			await vi.advanceTimersByTimeAsync(250);
			expect($aiStatus.get()).toEqual({
				llm: { state: "ready" },
				stt: { state: "ready" },
				loaded: true
			});
			expect(reads).toBe(2);
			unsubscribe();
		});

		it("stops retrying after the bounded backoff budget", async () => {
			captureListenerCalls();
			let reads = 0;
			mockInvoke({
				get_runtime_status: () => {
					reads++;
					throw new Error("ipc down");
				}
			});
			const unsubscribe = $aiStatus.subscribe(() => {});
			await settle();
			await vi.advanceTimersByTimeAsync(60_000);
			// one initial read plus one retry per backoff step, then
			// silence: no infinite polling
			expect(reads).toBe(7);
			expect(llmAvailability($aiStatus.get())).toBe("unknown");
			unsubscribe();
		});

		it("keeps live events after a failed read and prefers them over the retry's stale answer", async () => {
			const { handlers } = captureListenerCalls();
			let reads = 0;
			mockInvoke({
				get_runtime_status: () => {
					reads++;
					if (reads === 1) throw new Error("ipc down");
					// both engines moved on after the failed read
					return { llm: { state: "loading" }, stt: { state: "loading" } };
				}
			});
			const unsubscribe = $aiStatus.subscribe(() => {});
			await settle();
			handlers["llm-status"]!({ payload: { state: "ready" } });
			handlers["stt-status"]!({ payload: { state: "missing" } });
			// the events update the values but do not by themselves mark
			// the feed loaded
			expect($aiStatus.get().llm).toEqual({ state: "ready" });
			expect($aiStatus.get().stt).toEqual({ state: "missing" });
			expect($aiStatus.get().loaded).toBe(false);
			await vi.advanceTimersByTimeAsync(250);
			// the retry's snapshot is older than both events: they win
			expect($aiStatus.get()).toEqual({
				llm: { state: "ready" },
				stt: { state: "missing" },
				loaded: true
			});
			unsubscribe();
		});

		it("does not let one engine's event make the other engine's placeholder authoritative", async () => {
			const { handlers } = captureListenerCalls();
			let reads = 0;
			mockInvoke({
				get_runtime_status: () => {
					reads++;
					if (reads === 1) throw new Error("ipc down");
					return { llm: { state: "loading" }, stt: { state: "error" } };
				}
			});
			const unsubscribe = $aiStatus.subscribe(() => {});
			await settle();
			// only the llm engine ever reported: stt is still unknown
			handlers["llm-status"]!({ payload: { state: "ready" } });
			expect($aiStatus.get().loaded).toBe(false);
			await vi.advanceTimersByTimeAsync(250);
			// llm keeps its newer event value; stt is seeded by the
			// retry's real answer, not by the placeholder "missing"
			expect($aiStatus.get()).toEqual({
				llm: { state: "ready" },
				stt: { state: "error" },
				loaded: true
			});
			unsubscribe();
		});

		it("recovers a failed onMount start without an unhandled rejection", async () => {
			captureListenerCalls({ "llm-status": [new Error("ipc down")] });
			mockInvoke({
				get_runtime_status: () => ({ llm: { state: "ready" }, stt: { state: "ready" } })
			});
			const unsubscribe = $aiStatus.subscribe(() => {});
			await settle();
			await vi.advanceTimersByTimeAsync(250);
			expect($aiStatus.get().loaded).toBe(true);
			unsubscribe();
		});

		it("cancels pending retries when the last subscriber leaves", async () => {
			captureListenerCalls();
			let reads = 0;
			mockInvoke({
				get_runtime_status: () => {
					reads++;
					throw new Error("ipc down");
				}
			});
			const unsubscribe = $aiStatus.subscribe(() => {});
			await settle();
			expect(reads).toBe(1);
			unsubscribe();
			// force nanostores' delayed unmount now, while a retry is
			// still pending: the teardown must cancel its timer
			cleanStores($aiStatus);
			await vi.advanceTimersByTimeAsync(60_000);
			expect(reads).toBe(1);
			expect($aiStatus.get()).toEqual({
				llm: { state: "missing" },
				stt: { state: "missing" },
				loaded: false
			});
		});

		it("unlistens a start that finishes after teardown and does not duplicate listeners on restart", async () => {
			const handlers: Record<string, Handler> = {};
			const calls: string[] = [];
			const unlistened: string[] = [];
			const pending: Array<() => void> = [];
			vi.mocked(listen).mockImplementation(((name: string, handler: Handler) => {
				calls.push(`listen:${name}`);
				handlers[name] = handler;
				return new Promise<() => void>((resolve) => {
					pending.push(() =>
						resolve(() => {
							unlistened.push(name);
						})
					);
				});
			}) as unknown as typeof listen);
			mockInvoke({
				get_runtime_status: () => ({ llm: { state: "ready" }, stt: { state: "ready" } })
			});

			const first = $aiStatus.subscribe(() => {});
			await settle();
			first();
			// nanostores unmounts after a delay: the first start is torn
			// down while it still awaits its listeners
			await vi.advanceTimersByTimeAsync(1000);
			const second = $aiStatus.subscribe(() => {});
			await settle();
			pending.splice(0).forEach((release) => release());
			await settle();
			pending.splice(0).forEach((release) => release());
			await settle();
			// each lifecycle acquired each listener exactly once (no
			// duplicates), the torn-down one released exactly its own
			// pair and the live one finished the read
			expect(calls).toHaveLength(4);
			expect(calls.filter((c) => c === "listen:llm-status")).toHaveLength(2);
			expect(calls.filter((c) => c === "listen:stt-status")).toHaveLength(2);
			expect(unlistened).toEqual(["llm-status", "stt-status"]);
			expect(runtimeStatusCalls()).toHaveLength(1);
			expect($aiStatus.get().loaded).toBe(true);
			second();
		});

		it("ignores a read answer that resolves after teardown", async () => {
			captureListenerCalls();
			let answer!: (status: unknown) => void;
			mockInvoke({
				get_runtime_status: () => new Promise((res) => (answer = res))
			});
			const unsubscribe = $aiStatus.subscribe(() => {});
			await settle();
			unsubscribe();
			await vi.advanceTimersByTimeAsync(1000);
			answer({ llm: { state: "error" }, stt: { state: "error" } });
			await settle();
			// the stale answer must not repopulate a torn-down store
			expect($aiStatus.get()).toEqual({
				llm: { state: "missing" },
				stt: { state: "missing" },
				loaded: false
			});
		});
	});

	describe("llmAvailability", () => {
		const at = (state: AiStatus["llm"]["state"]): AiStatus => ({
			llm: { state },
			stt: { state: "ready" },
			loaded: true
		});

		it("separates loading from missing and error", () => {
			expect(llmAvailability(at("loading"))).toBe("loading");
			expect(llmAvailability(at("missing"))).toBe("missing");
			expect(llmAvailability(at("error"))).toBe("error");
		});

		it("treats a loaded or external model as ready", () => {
			expect(llmAvailability(at("ready"))).toBe("ready");
			expect(llmAvailability(at("external"))).toBe("ready");
		});

		it("only reports busy while loading", () => {
			expect(llmBusy(at("loading"))).toBe(true);
			expect(llmBusy(at("missing"))).toBe(false);
			expect(llmBusy(at("error"))).toBe(false);
		});

		it("treats a status without the loaded flag as loaded", () => {
			expect(llmAvailability({ llm: { state: "missing" }, stt: { state: "ready" } })).toBe(
				"missing"
			);
		});
	});
});
