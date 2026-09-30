import { normalizeApiError } from "./helpers";

/**
 * Friendly copy for known backend error shapes, with an action the user
 * can take. Unknown errors fall back to the raw (normalized) message.
 */
export function describeError(error: unknown): { message: string; action?: string } {
	const raw = normalizeApiError(error);
	if (raw.includes("not downloaded yet") || raw.includes("no STT")) {
		return {
			message: "Brainstory needs a speech model before it can transcribe you.",
			action: "Open Settings > AI Models to download one, or switch to typing with the keyboard button."
		};
	}
	if (raw.includes("a model is already loading")) {
		return {
			message: "The AI model is still loading.",
			action: "Give it a moment and try again."
		};
	}
	if (raw.includes("Speech model not downloaded")) {
		return {
			message: "No speech model is downloaded yet.",
			action: "Type your message instead, or download a model in Settings > AI Models."
		};
	}
	if (raw.includes("generation cancelled")) {
		return { message: "Generation cancelled." };
	}
	if (raw.includes("external endpoint") || raw.includes("endpoint did not return")) {
		return {
			message: "The external AI endpoint did not respond properly.",
			action: "Check the URL and key in Settings > AI Models, or fall back to the local model."
		};
	}
	return { message: raw };
}
