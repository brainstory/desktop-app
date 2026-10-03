import { normalizeApiError } from "@helpers/helpers";

/**
 * Transcription failures that a retry can never fix: configuration
 * problems, bad input, and explicit Apple Speech failures. Substrings of
 * the backend's exact rejections (src-tauri: commands/ai.rs transcribe,
 * stt.rs wav_to_samples / transcribe_external, stt_apple.rs).
 */
const PERMANENT_TRANSCRIPTION_ERRORS = [
	// no speech model (commands/ai.rs)
	"not downloaded yet",
	// input problems (commands/ai.rs)
	"expected raw audio bytes",
	"no audio received",
	"no audio captured",
	"audio capture too large",
	// WAV decoding (stt.rs)
	"invalid WAV",
	"unsupported WAV",
	"WAV contains no samples",
	"invalid sample rate",
	// misconfigured external endpoint (stt.rs)
	"invalid STT endpoint URL",
	// explicit Apple Speech choice surfaces real failures (stt_apple.rs)
	"Apple Speech transcription",
	"speech recognition permission was denied"
];

/** Retry predicate for transcribeApi: only transient transport hiccups
 * deserve a second attempt. */
export function isTransientTranscriptionError(error: unknown): boolean {
	const raw = normalizeApiError(error);
	return !PERMANENT_TRANSCRIPTION_ERRORS.some((marker) => raw.includes(marker));
}
