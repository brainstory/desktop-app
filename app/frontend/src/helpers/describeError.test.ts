import { describe, expect, it } from "vitest";
import { describeError } from "./describeError";

// The raw strings below are the backend's exact rejections (src-tauri).
describe("describeError", () => {
	it("labels a missing language model as an LLM problem, not a speech one", () => {
		const d = describeError(
			"Language model not downloaded yet. Open Settings > AI Models to download one."
		);
		expect(d.kind).toBe("llm-missing");
		expect(d.message).not.toMatch(/speech|transcribe/i);
		expect(d.action).toMatch(/Settings > AI Models/);
	});

	it("labels a missing speech model", () => {
		const d = describeError(
			"Speech model not downloaded yet. Open Settings > AI Models to download one."
		);
		expect(d.kind).toBe("stt-missing");
		expect(d.message).toMatch(/speech model/i);
		expect(d.action).toMatch(/typing/);
	});

	it("labels any other not-downloaded model generically", () => {
		expect(describeError("model not downloaded yet").kind).toBe("model-missing");
	});

	it("recognises a model that is still loading", () => {
		const d = describeError("a model is already loading - try again in a moment");
		expect(d.kind).toBe("model-loading");
		expect(d.action).toMatch(/moment/);
	});

	it("recognises a cancelled generation", () => {
		expect(describeError("generation cancelled")).toEqual({
			kind: "cancelled",
			message: "Generation cancelled."
		});
	});

	it("recognises external endpoint failures", () => {
		expect(describeError("external endpoint stalled (no data for 60s)").kind).toBe(
			"external-endpoint"
		);
		expect(
			describeError("endpoint did not return an SSE stream (content-type text/html): x").kind
		).toBe("external-endpoint");
		expect(describeError("external endpoint rejected credentials (401)").kind).toBe(
			"external-endpoint"
		);
	});

	it("recognises a denied speech-recognition permission", () => {
		const d = describeError(
			"speech recognition permission was denied - allow it in System Settings > Privacy & Security > Speech Recognition"
		);
		expect(d.kind).toBe("speech-permission");
		expect(d.action).toMatch(/Privacy & Security/);
	});

	it("recognises an empty recording", () => {
		expect(describeError("no audio captured").kind).toBe("no-audio");
		expect(describeError("no audio received").kind).toBe("no-audio");
	});

	it("falls back to the raw message for anything else", () => {
		expect(describeError(new Error("disk full"))).toEqual({
			kind: "unknown",
			message: "disk full"
		});
	});
});
