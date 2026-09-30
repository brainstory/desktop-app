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

/**
 * Kick off a streaming generation. Events (chunk/cumulative/status) arrive on
 * the returned channel; the invoke itself resolves once generation finishes.
 */
export function generateResponseStreamApi(
	messages: ChatMessage[],
	summarize = false,
	reactTo: string | null = null,
	reactToAuthor: string | null = null,
	reactToIsCurrentUser = false,
	chatType: string | null = null
): {
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
export async function generateResponseApi(
	messages: ChatMessage[],
	reactTo: string | null = null,
	reactToAuthor: string | null = null,
	reactToIsCurrentUser = false,
	chatType: string | null = null
): Promise<string> {
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
