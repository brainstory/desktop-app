import { useEffect } from "react";
import { TOPICS, CHAT_TYPE, ASK_A_DIFFERENT_QUESTION } from "@src/const";
import type { ChatMessage } from "@src/types";
import { getQuestionOfTheDay } from "@helpers/qotd";
import { getQueryParam, normalizeApiError } from "@helpers/helpers";
import type { RefObject, Dispatch, SetStateAction } from "react";

export function useIdeaIdFromUrl(
	hasMountedRef: RefObject<boolean>,
	setIdeaId: Dispatch<SetStateAction<string | undefined>>
): void {
	useEffect(() => {
		if (hasMountedRef.current) {
			return;
		}
		// this is for safeguarding against extra idea saves from weird edges cases
		// related to the ideaId once being in the query parameter but isn't anymore
		setIdeaId(getQueryParam("id") ?? undefined);
		hasMountedRef.current = true;
	}, [hasMountedRef, setIdeaId]);
}

/** Query params relevant to the first prompt (null/undefined = absent). */
export type FirstPromptParams = Record<"qotd" | "topic", string | null | undefined>;

/** The params for the current page URL. */
export function currentQueryParams(): FirstPromptParams {
	return { qotd: getQueryParam("qotd"), topic: getQueryParam("topic") };
}

/**
 * The assistant's opening message for a new chat.
 *
 * Keep in sync with the prompts: story_interview_system_message.txt,
 * story_interview_react_system_message.txt and
 * story_interview_context_system_message.txt paraphrase these opener
 * strings ("The first assistant message of this conversation has already
 * been sent...") - change them together.
 */
export function getFirstPrompt(
	chatType: string,
	params: FirstPromptParams = currentQueryParams()
): string {
	const isQotd = params.qotd != null;
	// Only a bare non-negative integer selects a topic: Number(null) is 0
	// and Number("") is 0, so a missing or empty param used to pin the
	// conversation to TOPICS[0]'s prompt instead of the default greeting.
	const raw = params.topic ?? null;
	const topic = raw !== null && /^\d+$/.test(raw) ? TOPICS[Number(raw)] : undefined;

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
	const role: ChatMessage["role"] = isUser ? "user" : "assistant";
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

/** What the backend rejects a generation with after cancel_generation
 * (llm.rs checks the cancel token between chunks). */
export const GENERATION_CANCELLED = "generation cancelled";

/** A user-initiated cancel: never retried, never shown as an error. */
export function isGenerationCancelled(error: unknown): boolean {
	return normalizeApiError(error).includes(GENERATION_CANCELLED);
}

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
type SuccessCallbacks = (message: string, structuredResult: unknown) => void | Promise<void>;

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
		console.error("streaming result failed", err);
		if (onError) {
			onError(err);
		}
	}
};

export function findMostRecentAssistantContent(currConversation: ChatMessage[]): string | null {
	for (let i = currConversation.length - 1; i >= 0; i--) {
		const message = currConversation[i]!;
		if (message.role === "assistant") {
			return message.content;
		}
	}
	return null;
}

export function findMostRecentUserContent(currConversation: ChatMessage[]): string | null {
	for (let i = currConversation.length - 1; i >= 0; i--) {
		const message = currConversation[i]!;
		if (message.role === "user") {
			return message.content;
		}
	}
	return null;
}

export interface TranscriptPair {
	question: string;
	answer?: string;
}

interface GroupTranscriptOptions {
	/** Render a placeholder answer for an in-flight transcription. */
	transcribing?: boolean;
	/** Drop a trailing question that nobody answered yet (it's shown in the main chat UI). */
	hideTrailingUnansweredQuestion?: boolean;
}

/**
 * Pair each assistant question with the user answer(s) that follow it,
 * walking the interleaved conversation instead of splitting it into two
 * arrays and zipping by index (which misaligns after "ask a different
 * question" or any repeated role).
 */
export function groupTranscript(
	conversation: ChatMessage[],
	{ transcribing = false, hideTrailingUnansweredQuestion = false }: GroupTranscriptOptions = {}
): TranscriptPair[] {
	const pairs: TranscriptPair[] = [];
	let current: TranscriptPair | null = null;

	for (const message of conversation) {
		if (message.role === "assistant") {
			if (current) {
				pairs.push(current);
			}
			current = { question: message.content };
		} else {
			if (message.content === ASK_A_DIFFERENT_QUESTION) {
				// meta message ("ask me a different question"), not a real answer
				continue;
			}
			if (current) {
				current.answer = current.answer
					? `${current.answer} ${message.content}`
					: message.content;
			}
		}
	}

	if (current) {
		if (transcribing && current.answer === undefined) {
			current.answer = "transcribing...";
			pairs.push(current);
		} else if (current.answer !== undefined || !hideTrailingUnansweredQuestion) {
			pairs.push(current);
		}
	}

	return pairs;
}
