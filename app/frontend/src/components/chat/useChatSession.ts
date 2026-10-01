/**
 * Chat session state machine: conversation state transitions, moderation
 * resend flow, and the generation calls (coach turn + final result).
 * Extracted from ChatSection.
 */

import { useCallback, useState } from "react";
import type { ChatMessage } from "@src/types";
import { CONVERSATION_STATE, ASK_A_DIFFERENT_QUESTION } from "@src/const";
import {
	handleStreamResult,
	addConversationMessage,
	removeLastConversationMessage,
	isGenerationCancelled
} from "@helpers/chat";
import { generateResponseApi, generateResponseStreamApi } from "@helpers/api/ai";
import { callApiWithRetry, normalizeApiError, isModerationError } from "@helpers/helpers";
import { markGettingStartedDone } from "@helpers/storage";

/** What became of a sent user message:
 * - sent: the coach answered
 * - flagged: moderation removed it from the conversation
 * - failed / cancelled: it stays in the conversation, unanswered */
export type CoachResponseOutcome = "sent" | "flagged" | "failed" | "cancelled";

export interface ChatSessionOptions {
	chatType: string;
	parentIdea?: {
		id: string;
		title?: string | null;
		summary?: string | null;
		creatorName?: string | null;
	};
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

	const [conversationState, setConversationState] = useState(CONVERSATION_STATE.Start);
	const [isUserResendRequired, setIsUserResendRequired] = useState(false);
	const [inappropriateUserTranscript, setInappropriateUserTranscript] = useState<string | null>(
		null
	);
	const [readyToSave, setReadyToSave] = useState(false);
	/** the summary stream finished: the result text is final */
	const [resultComplete, setResultComplete] = useState(false);

	/** Generate assistant response. NOT for the final outline result.
	 * Resolves with what happened to the user's message, so the composer
	 * only clears once the message was actually answered. */
	// stable per conversation: ChatRecorder/RecordButton keep it in memo
	// and effect dependencies
	const handleGetResponse = useCallback(async (): Promise<CoachResponseOutcome> => {
		setConversationState(CONVERSATION_STATE.WaitingForCoach);
		const apiCall = () =>
			generateResponseApi(
				currConversation,
				parentIdea?.summary,
				parentIdea?.creatorName ?? null,
				parentIdea ? parentIdea?.creatorName == null : false,
				chatType
			);
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
			setIsUserResendRequired(false);
			setInappropriateUserTranscript(null);
			return "sent";
		} catch (err) {
			if (isGenerationCancelled(err)) {
				// user-initiated: the message stays and the chat returns
				// to idle, no error banner
				return "cancelled";
			}
			if (isModerationError(err)) {
				const removedMessage = removeLastConversationMessage(
					currConversation,
					setCurrConversation
				);
				setInappropriateUserTranscript(removedMessage);
				setIsUserResendRequired(true);
				return "flagged";
			}
			onError(normalizeApiError(err));
			return "failed";
		} finally {
			setConversationState(CONVERSATION_STATE.Idle);
		}
	}, [currConversation, setCurrConversation, parentIdea, chatType, onError]);

	/** Generate idea summary result */
	const handleGetResult = () => {
		if (!ideaId) {
			onError("Still saving this session - try again in a moment.");
			return;
		}
		conversationEndCallbacks();
		setConversationState(CONVERSATION_STATE.FinishWithResult);
		setResultComplete(false);
		const resultFinishedCallbacks = async (
			result: string,
			structuredResult: unknown
		): Promise<void> => {
			setResultComplete(true);
			try {
				await saveResult(ideaId, currConversation, result, structuredResult);
				setReadyToSave(true);
			} catch (e) {
				onError(`Could not save your summary: ${normalizeApiError(e)}`);
				setConversationState(CONVERSATION_STATE.Idle);
				return;
			}
			if (fromGuideParam) {
				markGettingStartedDone();
			}
		};
		handleStreamResult(
			() =>
				generateResponseStreamApi(
					currConversation,
					true,
					parentIdea?.summary,
					parentIdea?.creatorName ?? null,
					parentIdea ? parentIdea?.creatorName == null : false,
					chatType
				),
			setResult,
			resultFinishedCallbacks,
			(err) => {
				// drop any partial summary: while a result is set the
				// finished-result view replaces the chat (and its banner)
				setResult("");
				setResultComplete(false);
				if (!isGenerationCancelled(err)) {
					onError(normalizeApiError(err));
				}
				setConversationState(CONVERSATION_STATE.Idle);
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
		setConversationState(CONVERSATION_STATE.ReadyToSendUserTranscript);
	};

	return {
		conversationState,
		setConversationState,
		isUserResendRequired,
		inappropriateUserTranscript,
		readyToSave,
		resultComplete,
		handleGetResponse,
		handleGetResult,
		askADifferentQuestion
	};
}
