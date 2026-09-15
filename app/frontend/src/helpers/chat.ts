import { useEffect } from "react";
import { TOPICS, CHAT_TYPE } from "@src/const";
import type { ChatMessage } from "@src/types";
import { getQuestionOfTheDay } from "@helpers/qotd";
import { getQueryParam } from "@helpers/helpers";
import type { RefObject, Dispatch, SetStateAction } from "react";

export function useIdeaIdFromUrl(
	hasMounted: RefObject<boolean>,
	setIdeaId: Dispatch<SetStateAction<string | undefined>>
): void {
	useEffect(() => {
		if (hasMounted.current) {
			return;
		}
		// this is for safeguarding against extra idea saves from weird edges cases
		// related to the ideaId once being in the query parameter but isn't anymore
		setIdeaId(getQueryParam("id") ?? undefined);
		hasMounted.current = true;
	}, []);
}

export function getFirstPrompt(chatType: string): string {
	const isQotd = getQueryParam("qotd") !== null;
	const topicIndex = Number(getQueryParam("topic"));
	const topic = Number.isInteger(topicIndex) ? TOPICS[topicIndex] : undefined;

	let firstPrompt = "Hi, how's it going? What's on your mind?";
	if (chatType === CHAT_TYPE.FEEDBACK) {
		firstPrompt = `Would you like to extend this idea, react to specific points, or walk through the idea with me point by point?`;
	} else if (chatType === CHAT_TYPE.DAILY_INTENT) {
		firstPrompt = "Walk me through how you want your day to go.";
	} else if (isQotd) {
		firstPrompt = getQuestionOfTheDay();
	} else if (topic) {
		firstPrompt = topic.prompt;
	}

	return firstPrompt;
}

type SetConversation = (next: ChatMessage[]) => void;

/**
 * Append a message to the conversation and return the new array. The new
 * array is returned (instead of invoking a callback with a guessed length)
 * so callers can threshold off the *real* new length synchronously instead
 * of a stale `conversation.length + 1` from a closing closure.
 */
export const addConversationMessage = (
	message: string,
	isUser: boolean,
	conversation: ChatMessage[],
	setConversation: SetConversation
): ChatMessage[] => {
	const role = isUser ? "user" : "assistant";
	const next = [...conversation, { role, content: message }];
	setConversation(next);
	return next;
};

export const removeLastConversationMessage = (
	conversation: ChatMessage[],
	setConversation: SetConversation
): string => {
	const removedMessage = conversation[conversation.length - 1];
	setConversation(conversation.slice(0, -1));
	return removedMessage?.content ?? "";
};

export interface StreamChunk {
	type: string;
	content?: string;
}

export interface GenerationResult {
	response?: string;
	request_message_tokens?: number;
	request_word_count?: number;
	structured_result?: unknown;
}

type UpdateMessage = (message: string) => void;
type SuccessCallbacks = (
	message: string,
	structuredResult: unknown
) => void | Promise<void>;

interface StreamHandle {
	channel: { onmessage: (event: StreamChunk) => void };
	invokePromise: Promise<GenerationResult>;
}

export const handleStreamResult = async (
	generateResultStream: () => StreamHandle,
	updateMessage: UpdateMessage,
	successCallbacks: SuccessCallbacks,
	onError?: (err: unknown) => void
): Promise<void> => {
	const { channel, invokePromise } = generateResultStream();
	let message = "";
	channel.onmessage = (event: StreamChunk) => {
		if (event.type === "chunk" && typeof event.content === "string") {
			message = message + event.content;
			updateMessage(message);
		}
		if (event.type === "cumulative" && typeof event.content === "string") {
			message = event.content;
			updateMessage(message);
		}
	};
	try {
		const result = await invokePromise;
		if (result?.response) {
			message = result.response;
			updateMessage(message);
		}
		await successCallbacks(message, result?.structured_result ?? null);
	} catch (err) {
		console.log("streaming result failed", err);
		if (onError) {
			onError(err);
		}
	}
};

export function findMostRecentAssistantContent(
	currConversation: ChatMessage[]
): string | null {
	for (let i = currConversation.length - 1; i >= 0; i--) {
		const message = currConversation[i];
		if (message.role === "assistant") {
			return message.content;
		}
	}
	return null;
}

export function findMostRecentUserContent(
	currConversation: ChatMessage[]
): string | null {
	for (let i = currConversation.length - 1; i >= 0; i--) {
		const message = currConversation[i];
		if (message.role === "user") {
			return message.content;
		}
	}
	return null;
}
