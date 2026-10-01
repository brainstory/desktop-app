/**
 * Chat session state machine: conversation state transitions, moderation
 * resend flow, and the generation calls (coach turn + final result).
 * Extracted from ChatSection.
 */

import { useCallback, useReducer } from "react";
import type { ChatMessage } from "@src/types";
import type { ParentIdea } from "@components/chat/types";
import { CONVERSATION_STATE, ASK_A_DIFFERENT_QUESTION } from "@src/const";
import {
	handleStreamResult,
	addConversationMessage,
	removeLastConversationMessage,
	isGenerationCancelled
} from "@helpers/chat";
import {
	generateResponseApi,
	generateResponseStreamApi,
	type GenerateOptions
} from "@helpers/api/ai";
import { callApiWithRetry, normalizeApiError, isModerationError } from "@helpers/helpers";
import { markGettingStartedDone } from "@helpers/storage";

/** What became of a sent user message:
 * - sent: the coach answered
 * - flagged: moderation removed it from the conversation
 * - failed / cancelled: it stays in the conversation, unanswered */
export type CoachResponseOutcome = "sent" | "flagged" | "failed" | "cancelled";

export interface ChatSessionOptions {
	chatType: string;
	parentIdea?: ParentIdea;
	ideaId?: string;
	fromGuideParam?: string | null;
	onError: (message: string) => void;
	conversationEndCallbacks: () => void;
	setSaveState: (state: string) => void;
	/** persist the final result (ordered after in-flight autosaves) */
	saveResult: (
		ideaId: string,
		conversation: ChatMessage[],
		result: string,
		structuredResult: unknown
	) => Promise<void>;
	setResult: (result: string) => void;
}

/** The feedback-flow generation options for a parent idea (none for a
 * regular chat). */
function reactionOptions(
	parentIdea: ParentIdea | undefined
): Pick<GenerateOptions, "reactTo" | "reactToAuthor" | "reactToIsCurrentUser"> {
	if (!parentIdea) return {};
	return {
		reactTo: parentIdea.summary ?? null,
		reactToAuthor: parentIdea.creatorName ?? null,
		reactToIsCurrentUser: parentIdea.creatorName == null
	};
}

/** Everything the session tracks besides the conversation itself. */
export interface ChatSessionState {
	conversationState: string;
	/** moderation removed the last message; the user must reword it */
	isUserResendRequired: boolean;
	inappropriateUserTranscript: string | null;
	/** the final summary is saved */
	readyToSave: boolean;
	/** the summary stream finished: the result text is final */
	resultComplete: boolean;
}

/** The events that move the session between CONVERSATION_STATE values. */
export type ChatSessionEvent =
	| { type: "transcriptionStarted" }
	| { type: "transcriptionFailed" }
	| { type: "userMessageReady" }
	| { type: "draftRestored"; lastRole: ChatMessage["role"] | undefined }
	| { type: "coachRequested" }
	| { type: "coachAnswered" }
	| { type: "coachFlagged"; transcript: string }
	| { type: "coachFailed" }
	| { type: "resultRequested" }
	| { type: "resultStreamed" }
	| { type: "resultSaved" }
	| { type: "resultSaveFailed" }
	| { type: "resultFailed" };

export const initialChatSessionState: ChatSessionState = {
	conversationState: CONVERSATION_STATE.Start,
	isUserResendRequired: false,
	inappropriateUserTranscript: null,
	readyToSave: false,
	resultComplete: false
};

export function chatSessionReducer(
	state: ChatSessionState,
	event: ChatSessionEvent
): ChatSessionState {
	switch (event.type) {
		case "transcriptionStarted":
			return { ...state, conversationState: CONVERSATION_STATE.TranscribingUser };
		case "transcriptionFailed":
		case "coachFailed":
			return { ...state, conversationState: CONVERSATION_STATE.Idle };
		case "userMessageReady":
			return { ...state, conversationState: CONVERSATION_STATE.ReadyToSendUserTranscript };
		case "draftRestored":
			// a draft that ends on the user's turn still owes them an answer
			return {
				...state,
				conversationState:
					event.lastRole === "user"
						? CONVERSATION_STATE.ReadyToSendUserTranscript
						: CONVERSATION_STATE.Idle
			};
		case "coachRequested":
			return { ...state, conversationState: CONVERSATION_STATE.WaitingForCoach };
		case "coachAnswered":
			return {
				...state,
				conversationState: CONVERSATION_STATE.Idle,
				isUserResendRequired: false,
				inappropriateUserTranscript: null
			};
		case "coachFlagged":
			return {
				...state,
				conversationState: CONVERSATION_STATE.Idle,
				isUserResendRequired: true,
				inappropriateUserTranscript: event.transcript
			};
		case "resultRequested":
			return {
				...state,
				conversationState: CONVERSATION_STATE.FinishWithResult,
				resultComplete: false
			};
		case "resultStreamed":
			return { ...state, resultComplete: true };
		case "resultSaved":
			return { ...state, readyToSave: true };
		case "resultSaveFailed":
			return { ...state, conversationState: CONVERSATION_STATE.Idle };
		case "resultFailed":
			return { ...state, conversationState: CONVERSATION_STATE.Idle, resultComplete: false };
	}
}

export function useChatSession(
	currConversation: ChatMessage[],
	setCurrConversation: (next: ChatMessage[]) => void,
	options: ChatSessionOptions
) {
	const {
		chatType,
		parentIdea,
		ideaId,
		fromGuideParam,
		onError,
		conversationEndCallbacks,
		saveResult,
		setResult
	} = options;

	const [state, dispatch] = useReducer(chatSessionReducer, initialChatSessionState);

	/** Generate assistant response. NOT for the final outline result.
	 * Resolves with what happened to the user's message, so the composer
	 * only clears once the message was actually answered. */
	// stable per conversation: ChatRecorder/RecordButton keep it in memo
	// and effect dependencies
	const handleGetResponse = useCallback(async (): Promise<CoachResponseOutcome> => {
		dispatch({ type: "coachRequested" });
		const apiCall = () =>
			generateResponseApi({
				messages: currConversation,
				chatType,
				...reactionOptions(parentIdea)
			});
		try {
			// a cancel must stay cancelled: retrying would start a fresh
			// generation with a new cancel token 500 ms later
			const message = await callApiWithRetry(
				apiCall,
				1,
				(err) => !isGenerationCancelled(err)
			);
			const isUser = false;
			addConversationMessage(message, isUser, currConversation, setCurrConversation);
			dispatch({ type: "coachAnswered" });
			return "sent";
		} catch (err) {
			if (isGenerationCancelled(err)) {
				// user-initiated: the message stays and the chat returns
				// to idle, no error banner
				dispatch({ type: "coachFailed" });
				return "cancelled";
			}
			if (isModerationError(err)) {
				const removedMessage = removeLastConversationMessage(
					currConversation,
					setCurrConversation
				);
				dispatch({ type: "coachFlagged", transcript: removedMessage });
				return "flagged";
			}
			onError(normalizeApiError(err));
			dispatch({ type: "coachFailed" });
			return "failed";
		}
	}, [currConversation, setCurrConversation, parentIdea, chatType, onError]);

	/** Generate idea summary result */
	const handleGetResult = () => {
		if (!ideaId) {
			onError("Still saving this session - try again in a moment.");
			return;
		}
		conversationEndCallbacks();
		dispatch({ type: "resultRequested" });
		const resultFinishedCallbacks = async (
			result: string,
			structuredResult: unknown
		): Promise<void> => {
			dispatch({ type: "resultStreamed" });
			try {
				await saveResult(ideaId, currConversation, result, structuredResult);
				dispatch({ type: "resultSaved" });
			} catch (e) {
				onError(`Could not save your summary: ${normalizeApiError(e)}`);
				dispatch({ type: "resultSaveFailed" });
				return;
			}
			if (fromGuideParam) {
				markGettingStartedDone();
			}
		};
		handleStreamResult(
			() =>
				generateResponseStreamApi({
					messages: currConversation,
					summarize: true,
					chatType,
					...reactionOptions(parentIdea)
				}),
			setResult,
			resultFinishedCallbacks,
			(err) => {
				// drop any partial summary: while a result is set the
				// finished-result view replaces the chat (and its banner)
				setResult("");
				if (!isGenerationCancelled(err)) {
					onError(normalizeApiError(err));
				}
				dispatch({ type: "resultFailed" });
			}
		);
	};

	const askADifferentQuestion = async () => {
		const isUser = true;
		addConversationMessage(
			ASK_A_DIFFERENT_QUESTION,
			isUser,
			currConversation,
			setCurrConversation
		);
		dispatch({ type: "userMessageReady" });
	};

	return {
		...state,
		dispatch,
		handleGetResponse,
		handleGetResult,
		askADifferentQuestion
	};
}
