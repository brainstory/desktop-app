import { useState, useEffect, useRef } from "react";
import { useStore } from "@nanostores/react";
import { $aiStatus, llmAvailability } from "@components/global/aiStatusStore";
import { CONVERSATION_STATE, MIN_CONVERSATION_LENGTH_BEFORE_SAVE, CHAT_TYPE } from "@src/const";
import { findMostRecentAssistantContent, getFirstPrompt, useIdeaIdFromUrl } from "@helpers/chat";
import type { ChatMessage } from "@src/types";
import { getIdeaApi } from "@helpers/api/idea";
import { getQueryParam } from "@helpers/helpers";

import ChatRecorder from "@components/chat/reusable/ChatRecorder";
import FinishedResultSection from "@components/chat/reusable/FinishedResultSection";
import ChatIdeaMainSection from "@components/chat/ChatIdeaMainSection";
import ChatFeedbackMainSection from "@components/chat/ChatFeedbackMainSection";
import ErrorSection from "@components/error/ErrorSection";
import { ChatErrorBanner } from "@components/chat/ChatErrorBanner";
import { ModelStatusNotice } from "@components/chat/ModelStatusNotice";
import { useChatSession } from "@components/chat/useChatSession";
import {
	useDraftLoader,
	useIdeaPersistence,
	type ChatFatalError
} from "@components/chat/useIdeaPersistence";

import { AssistantResponseText } from "@components/chat/AssistantResponseText";
import ChatTopBar from "./reusable/ChatTopBar";

interface ChatSectionProps {
	draftId?: string;
	dailyLogId?: string | null;
	/** chat mode (computed by the parent from the URL, so this module has
	 * no import-time window.location reads) */
	chatType?: string;
	/** ?parentId= query value, when giving feedback on an idea */
	parentIdParam?: string | null;
	/** ?topic= query value (guide entry) */
	fromGuideParam?: string | null;
	conversationEndCallbacks: () => void;
}

export function ChatSection({
	draftId,
	dailyLogId,
	chatType = CHAT_TYPE.ORIGINAL,
	parentIdParam = null,
	fromGuideParam = null,
	conversationEndCallbacks
}: ChatSectionProps) {
	const [result, setResult] = useState("");
	const [parentIdea, setParentIdea] = useState<
		| {
				id: string;
				title?: string | null;
				summary?: string | null;
				creatorName?: string | null;
		  }
		| undefined
	>();
	const [showTranscript, setShowTranscript] = useState(false);
	/** fatal load failure: an error section replaces the mic ui */
	const [fatalError, setFatalError] = useState<ChatFatalError | null>(null);
	/** error from the AI layer that is not the 469 resend case */
	const [aiError, setAiError] = useState<string | null>(null);

	const aiStatus = useStore($aiStatus);
	const modelAvailability = llmAvailability(aiStatus);
	const firstPrompt = getFirstPrompt(chatType);
	const [currConversation, setCurrConversation] = useState<ChatMessage[]>([
		{ role: "assistant", content: firstPrompt }
	]);
	const minConversationLenForCreateAndEnd =
		MIN_CONVERSATION_LENGTH_BEFORE_SAVE[chatType] ??
		MIN_CONVERSATION_LENGTH_BEFORE_SAVE.DEFAULT!;

	const fetchParentIdea = (parentId: string): void => {
		getIdeaApi(parentId)
			.then((res) => {
				document.title = `Feedback for "${res.title}"`;
				setParentIdea({ id: res.id, title: res.title, summary: res.summary });
			})
			.catch((err) => {
				console.error("Parent Idea not found with ID", parentId, err);
				setFatalError("parent-not-found");
			});
	};

	const hasMounted = useRef(false);

	const persistence = useIdeaPersistence(currConversation, result, {
		initialIdeaId: draftId,
		chatType,
		parentIdParam,
		dailyLogId,
		minLength: minConversationLenForCreateAndEnd,
		onError: setAiError,
		onFatalError: setFatalError,
		onParentIdea: fetchParentIdea
	});

	// session's setConversationState is stable (useState setter), but the
	// hook ordering requires declaring it after the loaders; a ref bridges
	const setConvStateRef = useRef<(s: string) => void>(() => {});
	useDraftLoader(persistence.ideaId, parentIdParam, {
		conversationLengthRef: persistence.conversationLengthRef,
		fetchedParentRef: persistence.fetchedParentRef,
		setIdeaId: persistence.setIdeaId,
		setCurrConversation,
		markPersisted: persistence.markPersisted,
		setConversationState: (s) => setConvStateRef.current(s),
		onParentIdea: fetchParentIdea,
		onFatalError: setFatalError
	});

	useIdeaIdFromUrl(hasMounted, (id) => persistence.setIdeaId(id));

	const session = useChatSession(currConversation, setCurrConversation, {
		chatType,
		parentIdea,
		ideaId: persistence.ideaId,
		fromGuideParam,
		onError: setAiError,
		conversationEndCallbacks,
		setSaveState: persistence.setSaveState,
		saveResult: persistence.saveResult,
		setResult
	});

	useEffect(() => {
		setConvStateRef.current = session.setConversationState;
	});

	if (result) {
		return (
			<FinishedResultSection
				ideaId={persistence.ideaId}
				result={result}
				parentSummary={parentIdea?.summary}
				readyForFinish={session.readyToSave}
			/>
		);
	} else if (fatalError === "draft-not-found") {
		return (
			<ErrorSection
				title="Draft idea not found"
				paragraphs={[
					"This draft no longer exists.",
					"If you did not mean to open a draft, start a new idea instead"
				]}
			/>
		);
	} else if (fatalError === "parent-not-found") {
		return (
			<ErrorSection
				title="Shared idea not found"
				paragraphs={[
					"The idea you were giving feedback on no longer exists in this library."
				]}
			/>
		);
	} else {
		const aiMessageContent = session.isUserResendRequired
			? session.inappropriateUserTranscript
			: findMostRecentAssistantContent(currConversation);
		const enableSkip =
			currConversation.length > 1 && session.conversationState === CONVERSATION_STATE.Idle;
		const mainSectionChildren = [
			<AssistantResponseText
				key="assistant-response"
				styleSetting={parentIdea && "feedback"}
				content={aiMessageContent}
				enableSkip={enableSkip}
				handleSkipQuestion={session.askADifferentQuestion}
				didFailToSend={session.isUserResendRequired}
			/>,
			<ChatRecorder
				key="chat-recorder"
				isCompressed={Boolean(parentIdea)}
				conversationState={session.conversationState}
				setConversationState={session.setConversationState}
				currConversation={currConversation}
				setCurrConversation={setCurrConversation}
				setSaveState={persistence.setSaveState}
				handleGetResponse={session.handleGetResponse}
				modelLoading={modelAvailability === "loading"}
			/>
		];

		return (
			<section className="wow">
				{aiError && (
					<ChatErrorBanner aiError={aiError} onDismiss={() => setAiError(null)} />
				)}
				<ModelStatusNotice availability={modelAvailability} error={aiStatus.llm.error} />
				<ChatTopBar
					parentIdea={parentIdea}
					showTranscript={showTranscript}
					setShowTranscript={setShowTranscript}
					leftButtonIcon={
						fromGuideParam && !getQueryParam("id") ? "arrow-back-outline" : null
					}
					leftButtonHref="/get-started"
					saveState={persistence.saveState}
				/>
				{parentIdea ? (
					<ChatFeedbackMainSection
						showTranscript={showTranscript}
						setShowTranscript={setShowTranscript}
						parentIdea={parentIdea}
						currConversation={currConversation}
						conversationState={session.conversationState}
						handleGetResult={session.handleGetResult}
						minConversationLenForEnd={minConversationLenForCreateAndEnd}
					>
						{mainSectionChildren}
					</ChatFeedbackMainSection>
				) : (
					<ChatIdeaMainSection
						showTranscript={showTranscript}
						setShowTranscript={setShowTranscript}
						currConversation={currConversation}
						conversationState={session.conversationState}
						handleGetResult={session.handleGetResult}
						minConversationLenForEnd={minConversationLenForCreateAndEnd}
					>
						{mainSectionChildren}
					</ChatIdeaMainSection>
				)}
			</section>
		);
	}
}
