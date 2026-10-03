import { normalizeApiError } from "./helpers";
import { isGenerationCancelled } from "./chat";

/** What kind of failure an error is, so callers can branch without
 * re-matching strings. */
export type ErrorKind =
	| "llm-missing"
	| "stt-missing"
	| "model-missing"
	| "model-loading"
	| "cancelled"
	| "external-endpoint"
	| "speech-permission"
	| "no-audio"
	| "unknown";

export interface DescribedError {
	kind: ErrorKind;
	message: string;
	action?: string;
}

/**
 * Friendly copy for known backend error shapes, with an action the user
 * can take. Unknown errors fall back to the raw (normalized) message.
 *
 * The matched strings are the backend's exact rejections (src-tauri:
 * commands/ai.rs, models/state.rs, llm.rs, stt_apple.rs, voice.rs). Order
 * matters: the specific "Speech/Language model not downloaded" checks
 * must run before the generic "not downloaded yet" fallback.
 */
export function describeError(error: unknown): DescribedError {
	const raw = normalizeApiError(error);
	if (raw.includes("Language model not downloaded")) {
		return {
			kind: "llm-missing",
			message: "No language model is downloaded yet.",
			action: "Open Settings > AI Models to download one, or connect an external AI server."
		};
	}
	if (raw.includes("Speech model not downloaded")) {
		return {
			kind: "stt-missing",
			message: "Brainstory needs a speech model before it can transcribe you.",
			action: "Open Settings > AI Models to download one, or switch to typing with the keyboard button."
		};
	}
	if (raw.includes("not downloaded yet")) {
		return {
			kind: "model-missing",
			message: "An AI model this needs is not downloaded yet.",
			action: "Open Settings > AI Models to download it."
		};
	}
	if (raw.includes("a model is already loading")) {
		return {
			kind: "model-loading",
			message: "The AI model is still loading.",
			action: "Give it a moment and try again."
		};
	}
	if (isGenerationCancelled(raw)) {
		return { kind: "cancelled", message: "Generation cancelled." };
	}
	if (raw.includes("external endpoint") || raw.includes("endpoint did not return")) {
		return {
			kind: "external-endpoint",
			message: "The external AI endpoint did not respond properly.",
			action: "Check the URL and key in Settings > AI Models, or fall back to the local model."
		};
	}
	if (raw.includes("speech recognition permission was denied")) {
		return {
			kind: "speech-permission",
			message: "Brainstory isn't allowed to use speech recognition.",
			action: "Allow it in System Settings > Privacy & Security > Speech Recognition, or switch to typing."
		};
	}
	if (raw.includes("no audio captured") || raw.includes("no audio received")) {
		return {
			kind: "no-audio",
			message: "No audio was captured.",
			action: "Check that the right microphone is selected and try again."
		};
	}
	return { kind: "unknown", message: raw };
}
