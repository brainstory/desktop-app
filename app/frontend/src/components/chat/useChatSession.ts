/**
 * Chat session state machine: conversation state transitions, moderation
 * resend flow, and the generation calls (coach turn + final result).
 * Extracted from ChatSection.
 */

import { useState } from "react";
import type { ChatMessage } from "@src/types";
import { CONVERSATION_STATE, ASK_A_DIFFERENT_QUESTION } from "@src/const";
import {
	handleStreamResult,
	addConversationMessage,
	removeLastConversationMessage
} from "@helpers/chat";
import { generateResponseApi, generateResponseStreamApi } from "@helpers/api/ai";
import { callApiWithRetry, normalizeApiError, isModerationError } from "@helpers/helpers";
import { markGettingStartedDone } from "@helpers/storage";
import { updateIdeaApi } from "@helpers/api/idea";

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
		setResult
	} = options;

	const [conversationState, setConversationState] = useState(CONVERSATION_STATE.Start);
	const [isUserResendRequired, setIsUserResendRequired] = useState(false);
	const [inappropriateUserTranscript, setInappropriateUserTranscript] = useState<string | null>(
		null
	);
	const [readyToSave, setReadyToSave] = useState(false);

	/** Generate assistant response. NOT for the final outline result. */
	const handleGetResponse = () => {
		setConversationState(CONVERSATION_STATE.WaitingForCoach);
		try {
			const apiCall = () =>
				generateResponseApi(
					currConversation,
					parentIdea?.summary,
					parentIdea?.creatorName ?? null,
					parentIdea ? parentIdea?.creatorName == null : false,
					chatType
				);
			callApiWithRetry(apiCall)
				.then((message) => {
					const isUser = false;
					addConversationMessage(message, isUser, currConversation, setCurrConversation);
					setIsUserResendRequired(false);
					setInappropriateUserTranscript(null);
				})
				.catch((err) => {
					if (isModerationError(err)) {
						const removedMessage = removeLastConversationMessage(
							currConversation,
							setCurrConversation
						);
						setInappropriateUserTranscript(removedMessage);
						setIsUserResendRequired(true);
					} else {
						onError(normalizeApiError(err));
					}
				})
				.finally(() => {
					setConversationState(CONVERSATION_STATE.Idle);
				});
		} catch (error) {
			console.error(error);
			setConversationState(CONVERSATION_STATE.Idle);
		}
	};

	/** Generate idea summary result */
	const handleGetResult = () => {
		if (!ideaId) {
			onError("Still saving this session - try again in a moment.");
			return;
		}
		conversationEndCallbacks();
		setConversationState(CONVERSATION_STATE.FinishWithResult);
		const resultFinishedCallbacks = async (
			result: string,
			structuredResult: unknown
		): Promise<void> => {
			try {
				await updateIdeaApi(ideaId, currConversation, result, structuredResult);
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
				onError(normalizeApiError(err));
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
		handleGetResponse,
		handleGetResult,
		askADifferentQuestion
	};
}
