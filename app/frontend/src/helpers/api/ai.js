import { invoke, Channel } from "@tauri-apps/api/core";

/**
 * @param {Blob} blobby WAV audio blob captured by the recorder
 * @returns {Promise<string>} text of transcribed audio
 */
export async function transcribeApi(blobby) {
	const bytes = new Uint8Array(await blobby.arrayBuffer());
	const response = await invoke("transcribe", bytes);
	return response.transcript;
}

/**
 * Kick off a streaming generation. Events (chunk/cumulative/status) arrive on
 * the returned channel; the invoke itself resolves once generation finishes.
 *
 * @returns {{ channel: Channel, invokePromise: Promise<object> }}
 */
export function generateResponseStreamApi(
	messages,
	summarize = false,
	reactTo = null,
	reactToAuthor = null,
	reactToIsCurrentUser = false,
	chatType = null
) {
	const channel = new Channel();
	const invokePromise = invoke("generate_streaming_response", {
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

/** @returns {Promise<string>} response text */
export async function generateResponseApi(
	messages,
	reactTo = null,
	reactToAuthor = null,
	reactToIsCurrentUser = false,
	chatType = null
) {
	const response = await invoke("generate_response", {
		messages,
		reactTo,
		reactToAuthor,
		reactToIsCurrentUser,
		chatType
	});
	return response?.response;
}
