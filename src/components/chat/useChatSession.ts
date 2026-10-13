/**
 * Chat session state machine: conversation state transitions, moderation
 * resend flow, and the generation calls (coach turn + final result).
 * Extracted from ChatSection.
 */

import { useCallback, useReducer, useRef } from "react";
import type { ChatMessage } from "@src/types";
import type { ChatErrorSource, ParentIdea } from "@components/chat/types";
import {
	CONVERSATION_STATE,
	ASK_A_DIFFERENT_QUESTION,
	type ChatSaveState,
	type ChatType,
	type ConversationState
} from "@src/const";
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
import { useUnmountSignal } from "@src/hooks/useUnmountSignal";

/** What became of a sent user message:
 * - sent: the coach answered
 * - flagged: moderation removed it from the conversation
 * - failed / cancelled: it stays in the conversation, unanswered */
export type CoachResponseOutcome = "sent" | "flagged" | "failed" | "cancelled";

export interface ChatSessionOptions {
	chatType: ChatType;
	parentIdea?: ParentIdea;
	ideaId?: string;
	fromGuideParam?: string | null;
	/** report an error; `source` defaults to "ai" */
	onError: (message: string, source?: ChatErrorSource) => void;
	conversationEndCallbacks: () => void;
	setSaveState: (state: ChatSaveState) => void;
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
	conversationState: ConversationState;
	/** moderation removed the last message; the user must reword it */
	isUserResendRequired: boolean;
	inappropriateUserTranscript: string | null;
	/** the final summary is saved */
	readyToSave: boolean;
	/** the summary stream finished: the result text is final */
	resultComplete: boolean;
	/** saving the finished summary failed (the summary is kept so the
	 * save can be retried) */
	resultSaveError: string | null;
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
	| { type: "userTurnRemoved" }
	| { type: "resultRequested" }
	| { type: "resultStreamed" }
	| { type: "resultSaved" }
	| { type: "resultSaveFailed"; message?: string }
	| { type: "resultSaveRetried" }
	| { type: "resultFailed" };

export const initialChatSessionState: ChatSessionState = {
	conversationState: CONVERSATION_STATE.Start,
	isUserResendRequired: false,
	inappropriateUserTranscript: null,
	readyToSave: false,
	resultComplete: false,
	resultSaveError: null
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
		case "userTurnRemoved":
			// the undone answer is gone (with the coach's reply to it):
			// nothing is pending, the previous question is on screen again
			return {
				...state,
				conversationState: CONVERSATION_STATE.Idle,
				isUserResendRequired: false,
				inappropriateUserTranscript: null
			};
		case "resultRequested":
			return {
				...state,
				conversationState: CONVERSATION_STATE.FinishWithResult,
				resultComplete: false,
				resultSaveError: null
			};
		case "resultStreamed":
			return { ...state, resultComplete: true };
		case "resultSaved":
			return { ...state, readyToSave: true, resultSaveError: null };
		case "resultSaveFailed":
			return {
				...state,
				conversationState: CONVERSATION_STATE.Idle,
				resultSaveError: event.message ?? "Could not save your summary."
			};
		case "resultSaveRetried":
			return { ...state, resultSaveError: null };
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
	// leaving the chat cancels a pending coach retry
	const unmountSignal = useUnmountSignal();

	/** Generate the assistant's next response for `conversation`. NOT for
	 * the final outline result. Resolves with what happened to the user's
	 * message, so the composer only clears once the message was actually
	 * answered. The conversation is a parameter (not read from the
	 * closure) so a freshly edited answer can be responded to in the same
	 * tick that set it. */
	// stable per conversation: ChatRecorder/RecordButton keep it in memo
	// and effect dependencies
	const respondTo = useCallback(
		async (conversation: ChatMessage[]): Promise<CoachResponseOutcome> => {
			dispatch({ type: "coachRequested" });
			const apiCall = () =>
				generateResponseApi({
					messages: conversation,
					chatType,
					...reactionOptions(parentIdea)
				});
			const signal = unmountSignal();
			try {
				// a cancel must stay cancelled: retrying would start a fresh
				// generation with a new cancel token 500 ms later
				const message = await callApiWithRetry(
					apiCall,
					1,
					(err) => !isGenerationCancelled(err),
					signal
				);
				const isUser = false;
				addConversationMessage(message, isUser, conversation, setCurrConversation);
				dispatch({ type: "coachAnswered" });
				return "sent";
			} catch (err) {
				if (signal.aborted) {
					// the chat is gone: nobody to show an error to
					return "cancelled";
				}
				if (isGenerationCancelled(err)) {
					// user-initiated: the message stays and the chat returns
					// to idle, no error banner
					dispatch({ type: "coachFailed" });
					return "cancelled";
				}
				if (isModerationError(err)) {
					const removedMessage = removeLastConversationMessage(
						conversation,
						setCurrConversation
					);
					dispatch({ type: "coachFlagged", transcript: removedMessage });
					return "flagged";
				}
				onError(normalizeApiError(err));
				dispatch({ type: "coachFailed" });
				return "failed";
			}
		},
		[setCurrConversation, parentIdea, chatType, onError, unmountSignal]
	);

	const handleGetResponse = useCallback(
		() => respondTo(currConversation),
		[respondTo, currConversation]
	);

	/** Index of the last user message, or -1. */
	const lastUserIndex = (conversation: ChatMessage[]): number => {
		for (let i = conversation.length - 1; i >= 0; i--) {
			if (conversation[i]!.role === "user") return i;
		}
		return -1;
	};

	/** True when the user's last answer can still be corrected: the
	 * exchange is settled (coach replied, nothing generating) and the
	 * last user message is a real answer (not "ask a different
	 * question"). */
	const canReviseLastAnswer = (() => {
		if (state.conversationState !== CONVERSATION_STATE.Idle || state.isUserResendRequired) {
			return false;
		}
		const idx = lastUserIndex(currConversation);
		return (
			idx >= 0 &&
			idx === currConversation.length - 2 &&
			currConversation[idx]!.content !== ASK_A_DIFFERENT_QUESTION
		);
	})();

	/** Replace the last answer and ask the coach again from there. A
	 * mis-transcribed answer poisons the conversation, so it must be
	 * fixable (and the reply based on it retired, not kept). */
	const editLastUserMessage = async (newText: string): Promise<void> => {
		if (state.conversationState !== CONVERSATION_STATE.Idle || state.isUserResendRequired) {
			return;
		}
		const idx = lastUserIndex(currConversation);
		if (idx < 0) return;
		const next = [
			...currConversation.slice(0, idx),
			{ role: "user" as const, content: newText }
		];
		setCurrConversation(next);
		await respondTo(next);
	};

	/** Undo the last answer and the coach's reply to it: the previous
	 * question is on screen again, ready to be answered (this time
	 * differently). */
	const removeLastUserTurn = (): void => {
		if (state.conversationState !== CONVERSATION_STATE.Idle || state.isUserResendRequired) {
			return;
		}
		const idx = lastUserIndex(currConversation);
		if (idx < 0) return;
		setCurrConversation(currConversation.slice(0, idx));
		dispatch({ type: "userTurnRemoved" });
	};

	/** the generated summary whose save failed, kept for a retry */
	const unsavedResultRef = useRef<{
		ideaId: string;
		conversation: ChatMessage[];
		result: string;
		structuredResult: unknown;
	} | null>(null);

	const persistResult = async (job: NonNullable<typeof unsavedResultRef.current>) => {
		unsavedResultRef.current = job;
		try {
			await saveResult(job.ideaId, job.conversation, job.result, job.structuredResult);
		} catch (e) {
			// the finished-result view replaces the chat (and its error
			// banner), so the failure is shown there, with a retry
			dispatch({ type: "resultSaveFailed", message: normalizeApiError(e) });
			return;
		}
		unsavedResultRef.current = null;
		dispatch({ type: "resultSaved" });
		if (fromGuideParam) {
			markGettingStartedDone();
		}
	};

	/** Retry saving a generated summary after a failed save. */
	const retrySaveResult = () => {
		const job = unsavedResultRef.current;
		if (!job) return;
		dispatch({ type: "resultSaveRetried" });
		void persistResult(job);
	};

	/** Generate idea summary result */
	const handleGetResult = () => {
		if (!ideaId) {
			onError("Still saving this session - try again in a moment.", "save");
			return;
		}
		conversationEndCallbacks();
		dispatch({ type: "resultRequested" });
		const resultFinishedCallbacks = async (
			result: string,
			structuredResult: unknown
		): Promise<void> => {
			dispatch({ type: "resultStreamed" });
			await persistResult({
				ideaId,
				conversation: currConversation,
				result,
				structuredResult
			});
		};
		// handleStreamResult never rejects: failures go to the onError
		// callback below
		void handleStreamResult(
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

	const askADifferentQuestion = (): void => {
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
		retrySaveResult,
		askADifferentQuestion,
		canReviseLastAnswer,
		editLastUserMessage,
		removeLastUserTurn
	};
}
