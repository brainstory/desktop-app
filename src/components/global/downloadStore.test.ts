import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanStores } from "nanostores";

import {
	$downloads,
	clearDownloadFailure,
	formatDownloadError,
	markDownloadStarting,
	seedDownloads
} from "./downloadStore";
import type { DownloadEvent } from "@src/tauri/generated/DownloadEvent";
import { captureListen, mockedListen } from "@src/test/mock-tauri";

/** Read the store for a test (which mounts its onMount listener) and
 * hand back an emitter of typed events. */
function subscribeCapturing(): {
	emit: (payload: DownloadEvent) => void;
	unsubscribe: () => void;
} {
	const { emit, handlers } = captureListen();
	const unsubscribe = $downloads.subscribe(() => {});
	// the mock runs synchronously inside the mount
	const downloadHandlers = handlers;
	if (downloadHandlers.length === 0) {
		throw new Error("the feed did not register its listener");
	}
	return { emit: (payload) => emit("model-download", payload), unsubscribe };
}

const IDLE = { active: {}, paused: {}, failures: {} };

afterEach(() => {
	mockedListen.mockReset();
	mockedListen.mockImplementation(async () => () => {});
	$downloads.set(IDLE);
	// nanostores delays the real unmount by a second (churn guard):
	// force it so every test starts with an unmounted store
	cleanStores($downloads);
	vi.useRealTimers();
});

describe("formatDownloadError", () => {
	it("turns a refused request into the status it carried", () => {
		expect(formatDownloadError("download failed with status 403")).toBe(
			"The download server refused the request (403). Try again later."
		);
	});

	it.each([
		["download stalled (no data for 60s)", /The download stalled/],
		["download timed out waiting for headers", /didn't respond in time/],
		["download interrupted (connection reset)", /connection dropped/],
		["download incomplete (got 5 of 9 bytes)", /stopped early/],
		["download size mismatch (got 5 bytes, expected 9)", /different file than expected/],
		["download failed its integrity check (sha256 abc)", /didn't match its checksum/],
		["download request failed: dns error", /Couldn't reach the download server/],
		["download cancelled", /Download cancelled — nothing was kept\./]
	])("maps %s to plain language", (raw, expected) => {
		expect(formatDownloadError(raw)).toMatch(expected);
	});

	it("falls through with the original text when nothing matches", () => {
		expect(formatDownloadError("  disk full  ")).toBe("Download failed: disk full");
	});
});

describe("the download event feed", () => {
	it("tracks a progress event, then a paused one, then the resume", () => {
		const { emit, unsubscribe } = subscribeCapturing();

		emit({ modelId: "m1", kind: "progress", pct: 41.2, label: "Model 1" });
		expect($downloads.get().active["m1"]).toEqual({
			pct: 41.2,
			label: "Model 1"
		});

		// speed/ETA ride along when the backend computed them
		emit({
			modelId: "m1",
			kind: "progress",
			pct: 44,
			bytesPerSec: 5_500_000,
			etaSecs: 40
		});
		expect($downloads.get().active["m1"]).toEqual({
			pct: 44,
			label: "Model 1",
			bytesPerSec: 5_500_000,
			etaSecs: 40
		});

		emit({ modelId: "m1", kind: "paused", pct: 44, label: "Model 1" });
		expect($downloads.get().active["m1"]).toBeUndefined();
		expect($downloads.get().paused["m1"]).toEqual({ pct: 44, label: "Model 1" });

		// resuming: progress supersedes the paused marker
		emit({ modelId: "m1", kind: "progress", pct: 46, label: "Model 1" });
		expect($downloads.get().paused["m1"]).toBeUndefined();
		expect($downloads.get().active["m1"]).toEqual({ pct: 46, label: "Model 1" });

		// done clears everything for the model
		emit({ modelId: "m1", kind: "done", label: "Model 1" });
		expect($downloads.get().active["m1"]).toBeUndefined();
		expect($downloads.get().paused["m1"]).toBeUndefined();
		unsubscribe();
	});

	it("records an unknown size as the indeterminate sentinel", () => {
		// the typed event always carries a pct; the -1 sentinel is the
		// optimistic seed a button press plants before the first event
		const unsubscribe = $downloads.subscribe(() => {});
		markDownloadStarting("m1", "Model 1");
		expect($downloads.get().active["m1"]).toEqual({ pct: -1, label: "Model 1" });
		unsubscribe();
	});

	it("a failure reads in plain language and clears progress", () => {
		const { emit, unsubscribe } = subscribeCapturing();
		emit({ modelId: "m1", kind: "progress", pct: 10, label: "Model 1" });
		emit({
			modelId: "m1",
			kind: "error",
			message: "download stalled (no data for 60s) - please retry"
		});
		expect($downloads.get().active["m1"]).toBeUndefined();
		expect($downloads.get().failures["m1"]).toEqual({
			label: "Model 1",
			message:
				"The download stalled — no data arrived for a while. Try again; it resumes where it left off."
		});

		// a retry event wipes the recorded failure
		emit({ modelId: "m1", kind: "progress", pct: 1, label: "Model 1" });
		expect($downloads.get().failures["m1"]).toBeUndefined();
		unsubscribe();
	});

	it("a cancel is not an error: it clears progress and records nothing", () => {
		const { emit, unsubscribe } = subscribeCapturing();
		emit({ modelId: "m1", kind: "progress", pct: 80, label: "Model 1" });
		emit({ modelId: "m1", kind: "cancelled", label: "Model 1" });
		expect($downloads.get()).toEqual({ active: {}, paused: {}, failures: {} });
		unsubscribe();
	});

	it("ignores channel-shaped events (no model id) and activations", () => {
		const { emit, unsubscribe } = subscribeCapturing();
		// the per-download channel variant carries no model id; the
		// global event with the model id follows immediately
		emit({ kind: "progress", pct: 10 });
		emit({ kind: "done" });
		emit({ modelId: "m1", kind: "activated", label: "Model 1" });
		expect($downloads.get()).toEqual(IDLE);
		unsubscribe();
	});

	it("clearDownloadFailure drops one recorded failure", () => {
		const { emit, unsubscribe } = subscribeCapturing();
		emit({ modelId: "m1", kind: "error", message: "boom", label: "Model 1" });
		emit({ modelId: "m2", kind: "error", message: "boom", label: "Model 2" });
		clearDownloadFailure("m1");
		expect(Object.keys($downloads.get().failures)).toEqual(["m2"]);
		// an unknown id is a harmless no-op
		clearDownloadFailure("ghost");
		unsubscribe();
	});
});

describe("seedDownloads", () => {
	it("fills in downloads the store does not track yet", () => {
		$downloads.set({
			active: { fresh: { pct: 30, label: "Fresh" } },
			paused: {},
			failures: {}
		});
		seedDownloads(
			{ fresh: { pct: 10, label: "Fresh" }, other: { pct: 5, label: "Other" } },
			{ stopped: { pct: 60, label: "Stopped" } }
		);
		// the live event pct is fresher than the listing snapshot
		expect($downloads.get().active["fresh"]).toEqual({ pct: 30, label: "Fresh" });
		expect($downloads.get().active["other"]).toEqual({ pct: 5, label: "Other" });
		expect($downloads.get().paused["stopped"]).toEqual({ pct: 60, label: "Stopped" });

		// an id that resumed is never seeded back into paused
		seedDownloads({}, { other: { pct: 5, label: "Other" } });
		expect($downloads.get().paused["other"]).toBeUndefined();
	});

	it("does nothing when there is nothing to add", () => {
		seedDownloads({}, {});
		expect($downloads.get()).toEqual(IDLE);
	});
});

describe("the store lifecycle (onMount)", () => {
	it("shares one listener and resets the store when the last reader leaves", () => {
		vi.useFakeTimers();
		mockedListen.mockImplementation(async () => () => {});
		const first = $downloads.subscribe(() => {});
		const second = $downloads.subscribe(() => {});
		// one registration for both readers (the shared feed)
		expect(mockedListen).toHaveBeenCalledTimes(1);

		$downloads.set({ active: { m: { pct: 1, label: "M" } }, paused: {}, failures: {} });
		first();
		// the second reader keeps the feed alive: state survives
		expect($downloads.get().active["m"]).toBeDefined();
		second();
		// nanostores delays the unmount (a churn guard for pages that
		// swap readers within a second): after it, the feed resets
		vi.advanceTimersByTime(1000);
		expect($downloads.get()).toEqual(IDLE);
	});

	it("a reader that leaves before registration resolves releases the listener itself", async () => {
		vi.useFakeTimers();
		const unlisten = vi.fn();
		let release!: () => void;
		mockedListen.mockImplementation(
			() =>
				new Promise((resolve) => {
					release = () => resolve(unlisten);
				})
		);
		const unsubscribe = $downloads.subscribe(() => {});
		unsubscribe();
		// the unmount's teardown runs after the churn-guard delay, then
		// the registration that resolves afterwards releases itself
		vi.advanceTimersByTime(1000);
		release();
		await Promise.resolve();
		await Promise.resolve();
		expect(unlisten).toHaveBeenCalled();
	});
});
