import { afterEach, describe, expect, it, vi } from "vitest";
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
