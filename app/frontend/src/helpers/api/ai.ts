import { invoke, Channel } from "@tauri-apps/api/core";
import { COMMANDS } from "@src/tauri/commands";
import type { ChatMessage } from "@src/types";
import type { GenerationResult } from "@helpers/chat";

/**
 * @param blob WAV audio blob captured by the recorder
 * @returns text of transcribed audio
 */
export async function transcribeApi(blob: Blob): Promise<string> {
	const bytes = new Uint8Array(await blob.arrayBuffer());
	const response = await invoke<{ transcript: string }>(COMMANDS.transcribe, bytes);
	return response.transcript;
}

/** What to generate from: the conversation plus, for a feedback chat,
 * the idea being reacted to. */
export interface GenerateOptions {
	messages: ChatMessage[];
	chatType?: string | null;
	/** the parent idea's document a feedback chat reacts to */
	reactTo?: string | null;
	/** who wrote it (imported ideas only) */
	reactToAuthor?: string | null;
	/** the user wrote the parent idea themselves */
	reactToIsCurrentUser?: boolean;
}

/**
 * Kick off a streaming generation. Events (chunk/cumulative/status) arrive on
 * the returned channel; the invoke itself resolves once generation finishes.
 */
export function generateResponseStreamApi({
	messages,
	summarize = false,
	reactTo = null,
	reactToAuthor = null,
	reactToIsCurrentUser = false,
	chatType = null
}: GenerateOptions & {
	/** generate the final summary instead of the next coach turn */
	summarize?: boolean;
}): {
	channel: Channel;
	invokePromise: Promise<GenerationResult>;
} {
	const channel = new Channel();
	const invokePromise = invoke<GenerationResult>(COMMANDS.generateStreamingResponse, {
		messages,
		summarize,
		reactTo,
		reactToAuthor,
		reactToIsCurrentUser,
		chatType,
		onEvent: channel
	});
	return {
		channel,
		invokePromise
	};
}

/** @returns response text */
export async function generateResponseApi({
	messages,
	reactTo = null,
	reactToAuthor = null,
	reactToIsCurrentUser = false,
	chatType = null
}: GenerateOptions): Promise<string> {
	const response = await invoke<GenerationResult>(COMMANDS.generateResponse, {
		messages,
		reactTo,
		reactToAuthor,
		reactToIsCurrentUser,
		chatType
	});
	return response?.response ?? "";
}

/**
 * Cancel the in-flight generation. The backend checks the token between
 * chunks, so the invoke rejects with "generation cancelled" shortly after.
 */
export function cancelGenerationApi(): Promise<void> {
	return invoke(COMMANDS.cancelGeneration);
}
