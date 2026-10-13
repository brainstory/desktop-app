import { afterEach, describe, expect, it, vi } from "vitest";
import { listen, type EventCallback } from "@tauri-apps/api/event";

import {
	$downloads,
	clearDownloadFailure,
	formatDownloadError,
	seedDownloads,
	subscribeDownloads,
	type ModelDownloadEvent
} from "./downloadStore";
import { EVENTS } from "@src/tauri/commands";

/** Subscribe for a test and hand back the backend handler it registered. */
function subscribeCapturing(): {
	handler: EventCallback<unknown>;
	unsubscribe: () => void;
} {
	let captured: EventCallback<unknown> | undefined;
	vi.mocked(listen).mockImplementation(async (event, h) => {
		if (event === EVENTS.modelDownload) captured = h;
		return () => {};
	});
	const unsubscribe = subscribeDownloads();
	// the mock runs synchronously inside subscribeDownloads
	if (!captured) {
		throw new Error("the feed did not register its listener");
	}
	return { handler: captured, unsubscribe };
}

function emit(handler: EventCallback<unknown>, payload: ModelDownloadEvent): void {
	handler({ event: EVENTS.modelDownload, id: 0, payload });
}

const IDLE = { active: {}, paused: {}, failures: {} };

afterEach(() => {
	vi.mocked(listen).mockReset();
	vi.mocked(listen).mockImplementation(async () => () => {});
	$downloads.set(IDLE);
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
		const { handler, unsubscribe } = subscribeCapturing();

		emit(handler, { modelId: "m1", kind: "progress", pct: 41.2, label: "Model 1" });
		expect($downloads.get().active["m1"]).toEqual({
			pct: 41.2,
			label: "Model 1"
		});

		// speed/ETA ride along when the backend computed them
		emit(handler, {
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

		emit(handler, { modelId: "m1", kind: "paused", pct: 44 });
		expect($downloads.get().active["m1"]).toBeUndefined();
		expect($downloads.get().paused["m1"]).toEqual({ pct: 44, label: "Model 1" });

		// resuming: progress supersedes the paused marker
		emit(handler, { modelId: "m1", kind: "progress", pct: 46 });
		expect($downloads.get().paused["m1"]).toBeUndefined();
		expect($downloads.get().active["m1"]).toEqual({ pct: 46, label: "Model 1" });

		// done clears everything for the model
		emit(handler, { modelId: "m1", kind: "done" });
		expect($downloads.get().active["m1"]).toBeUndefined();
		expect($downloads.get().paused["m1"]).toBeUndefined();
		unsubscribe();
	});

	it("records an unknown size as the indeterminate sentinel", () => {
		const { handler, unsubscribe } = subscribeCapturing();
		emit(handler, { modelId: "m1", kind: "progress" });
		expect($downloads.get().active["m1"]).toEqual({ pct: -1, label: "m1" });
		unsubscribe();
	});

	it("a failure reads in plain language and clears progress", () => {
		const { handler, unsubscribe } = subscribeCapturing();
		emit(handler, { modelId: "m1", kind: "progress", pct: 10, label: "Model 1" });
		emit(handler, {
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
		emit(handler, { modelId: "m1", kind: "progress", pct: 1 });
		expect($downloads.get().failures["m1"]).toBeUndefined();
		unsubscribe();
	});

	it("a cancel is not an error: it clears progress and records nothing", () => {
		const { handler, unsubscribe } = subscribeCapturing();
		emit(handler, { modelId: "m1", kind: "progress", pct: 80 });
		emit(handler, { modelId: "m1", kind: "cancelled" });
		expect($downloads.get()).toEqual({ active: {}, paused: {}, failures: {} });
		unsubscribe();
	});

	it("ignores events without a model id or a kind", () => {
		const { handler, unsubscribe } = subscribeCapturing();
		emit(handler, { kind: "progress", pct: 10 });
		emit(handler, { modelId: "m1" });
		emit(handler, { modelId: "m1", kind: "activated" });
		expect($downloads.get()).toEqual(IDLE);
		unsubscribe();
	});

	it("clearDownloadFailure drops one recorded failure", () => {
		const { handler, unsubscribe } = subscribeCapturing();
		emit(handler, { modelId: "m1", kind: "error", message: "boom" });
		emit(handler, { modelId: "m2", kind: "error", message: "boom" });
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

describe("subscribeDownloads", () => {
	it("shares one listener and resets the store when the last subscriber leaves", async () => {
		vi.mocked(listen).mockImplementation(async () => () => {});
		const first = subscribeDownloads();
		const second = subscribeDownloads();
		// one registration for both subscribers (the shared feed)
		expect(vi.mocked(listen)).toHaveBeenCalledTimes(1);

		$downloads.set({ active: { m: { pct: 1, label: "M" } }, paused: {}, failures: {} });
		first();
		// the second subscriber keeps the feed alive: state survives
		expect($downloads.get().active["m"]).toBeDefined();
		second();
		expect($downloads.get()).toEqual(IDLE);
	});

	it("a subscriber that leaves before registration resolves releases the listener itself", async () => {
		const unlisten = vi.fn();
		let release!: () => void;
		vi.mocked(listen).mockImplementation(
			() =>
				new Promise((resolve) => {
					release = () => resolve(unlisten);
				})
		);
		const unsubscribe = subscribeDownloads();
		unsubscribe();
		// the registration resolves after the subscriber left
		release();
		await Promise.resolve();
		await Promise.resolve();
		expect(unlisten).toHaveBeenCalled();
	});
});
