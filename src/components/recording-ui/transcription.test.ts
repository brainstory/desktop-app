import { describe, expect, it } from "vitest";
import { isTransientTranscriptionError } from "./transcription";

describe("isTransientTranscriptionError", () => {
	// exact backend rejections (src-tauri) that a retry can never fix
	it.each([
		"Speech model not downloaded yet. Open Settings > AI Models to download one.",
		"expected raw audio bytes",
		"no audio received",
		"no audio captured",
		"audio capture too large (40 MB, limit 32 MB)",
		"invalid WAV: unexpected EOF",
		"invalid WAV samples: bad data",
		"unsupported WAV bit depth (8-bit integer)",
		"WAV contains no samples",
		"invalid STT endpoint URL 'localhost' (include http:// or https://)",
		"Apple Speech transcription failed: analyzer error",
		"Apple Speech transcription requires macOS 26 or newer",
		"speech recognition permission was denied - allow it in System Settings"
	])("does not retry %s", (message) => {
		expect(isTransientTranscriptionError(message)).toBe(false);
		expect(isTransientTranscriptionError(new Error(message))).toBe(false);
	});

	it.each([
		"request failed: connection reset by peer",
		"external STT error (503 Service Unavailable)",
		"transcription failed: whisper state busy"
	])("retries %s", (message) => {
		expect(isTransientTranscriptionError(message)).toBe(true);
	});
});
