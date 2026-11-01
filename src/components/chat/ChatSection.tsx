import { useCallback, useState, useRef, useEffect } from "react";
import { useStore } from "@nanostores/react";
import { $aiStatus, llmAvailability, sttAvailability } from "@components/global/aiStatusStore";
import {
	CONVERSATION_STATE,
	MIN_CONVERSATION_LENGTH_BEFORE_SAVE,
	CHAT_TYPE,
	type ChatType
} from "@src/const";
import { findMostRecentAssistantContent, getFirstPrompt } from "@helpers/chat";
import type { ChatMessage } from "@src/types";
import type { ChatErrorSource, ParentIdea } from "@components/chat/types";
import { getIdeaApi } from "@helpers/api/idea";
import { getQueryParam } from "@helpers/helpers";
import { QUERY_PARAMS } from "@src/tauri/commands";

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
import LastAnswerActions from "@components/chat/reusable/LastAnswerActions";
import ChatTopBar from "./reusable/ChatTopBar";

/** Astro's client-router before-preparation event (the subset used
 * here; the full shape lives in astro:transitions/client). Wrapping or
 * calling `loader` performs the prepared swap. */
interface BeforePreparationEvent extends Event {
	/** the destination URL */
	to: string;
	loader: () => Promise<void> | void;
}

interface ChatSectionProps {
	draftId?: string;
	dailyLogId?: string | null;
	/** today's daily-intent status is still being read: recording waits */
	dailyStatusLoading?: boolean;
	/** chat mode (computed by the parent from the URL, so this module has
	 * no import-time window.location reads) */
	chatType?: ChatType;
	/** ?parentId= query value, when giving feedback on an idea */
	parentIdParam?: string | null;
	/** ?topic= query value (guide entry) */
	fromGuideParam?: string | null;
	conversationEndCallbacks: () => void;
}

export function ChatSection({
	draftId,
	dailyLogId,
	dailyStatusLoading = false,
	chatType = CHAT_TYPE.ORIGINAL,
	parentIdParam = null,
	fromGuideParam = null,
	conversationEndCallbacks
}: ChatSectionProps) {
	const [result, setResult] = useState("");
	const [parentIdea, setParentIdea] = useState<ParentIdea | undefined>();
	const [showTranscript, setShowTranscript] = useState(false);
	/** an early wrap-up is creating/saving the session before summarizing */
	const [isWrappingUp, setIsWrappingUp] = useState(false);
	/** the early wrap-up waits for the create: one-shot, cleared when the
	 * create fails (a later unrelated create must not finish the session) */
	const finishAfterCreateRef = useRef(false);
	/** fatal load failure: an error section replaces the mic ui */
	const [fatalError, setFatalError] = useState<ChatFatalError | null>(null);
	/** an AI error (other than the moderation resend case) or a save error */
	const [chatError, setChatError] = useState<{
		message: string;
		source: ChatErrorSource;
	} | null>(null);
	const onSessionError = useCallback(
		(message: string, source: ChatErrorSource = "ai") => setChatError({ message, source }),
		[]
	);
	const onSaveError = useCallback((message: string) => {
		setChatError({ message, source: "save" });
		// a failed create must release the early wrap-up button instead of
		// leaving it stuck on "Wrapping up…" - and must not arm a summary
		// that fires after some later, unrelated create lands
		finishAfterCreateRef.current = false;
		setIsWrappingUp(false);
	}, []);

	const aiStatus = useStore($aiStatus);
	const modelAvailability = llmAvailability(aiStatus);
	const speechAvailability = sttAvailability(aiStatus);
	// "Missing" and "error" block the input that would only fail; "unknown"
	// (no status yet) never blocks. "Loading" keeps its dedicated copy.
	const llmBlocked = modelAvailability === "missing" || modelAvailability === "error";
	const sttBlocked = speechAvailability === "missing" || speechAvailability === "error";

	// computed once: the prompt only matters for the initial message, so
	// the lazy initializer keeps it out of every later render
	const [currConversation, setCurrConversation] = useState<ChatMessage[]>(() => [
		{ role: "assistant", content: getFirstPrompt(chatType) }
	]);
	// per-type minimum (daily-intent sessions need less), typed so the
	// DEFAULT fallback needs no non-null assertion
	const minConversationLenForCreateAndEnd =
		MIN_CONVERSATION_LENGTH_BEFORE_SAVE[chatType] ??
		MIN_CONVERSATION_LENGTH_BEFORE_SAVE.DEFAULT;

	const fetchParentIdea = (parentId: string): void => {
		getIdeaApi(parentId)
			.then((res) => {
				document.title =
					chatType === CHAT_TYPE.CONTINUATION
						? `Continue thinking about "${res.title}"`
						: `Feedback for "${res.title}"`;
				// creatorName is only set on ideas imported from someone else;
				// without it the model is told the user wrote the idea
				setParentIdea({
					id: res.id,
					title: res.title,
					summary: res.summary,
					creatorName: res.creatorName
				});
			})
			.catch((err) => {
				console.error("Parent Idea not found with ID", parentId, err);
				setFatalError("parent-not-found");
			});
	};

	const persistence = useIdeaPersistence(currConversation, result, {
		initialIdeaId: draftId,
		chatType,
		parentIdParam,
		dailyLogId,
		minLength: minConversationLenForCreateAndEnd,
		onError: onSaveError,
		onFatalError: setFatalError,
		onParentIdea: fetchParentIdea
	});

	const session = useChatSession(currConversation, setCurrConversation, {
		chatType,
		parentIdea,
		ideaId: persistence.ideaId,
		fromGuideParam,
		onError: onSessionError,
		conversationEndCallbacks,
		setSaveState: persistence.setSaveState,
		saveResult: persistence.saveResult,
		setResult
	});
	const { dispatch } = session;

	const draftStatus = useDraftLoader(persistence.ideaId, parentIdParam, {
		conversationLengthRef: persistence.conversationLengthRef,
		fetchedParentRef: persistence.fetchedParentRef,
		createdIdeaIdRef: persistence.createdIdeaIdRef,
		setIdeaId: persistence.setIdeaId,
		setCurrConversation,
		markPersisted: persistence.markPersisted,
		onDraftRestored: (lastRole) => dispatch({ type: "draftRestored", lastRole }),
		onParentIdea: fetchParentIdea,
		onFatalError: setFatalError
	});

	// the specific members the interception/finish paths depend on: a
	// whole-object dependency re-runs them on every streamed token (see
	// the memo comments in the hooks)
	const { ideaId: sessionIdeaId, hasUnsavedChanges, whenIdle, requestCreate } = persistence;
	const { handleGetResult } = session;

	// Cmd/Ctrl+Enter ends the session once there is something to end
	// with (the same condition that shows a finish button)
	// (only skips so far = nothing to save: the create would never run and
	// a wrap-up would wait on it forever)
	const hasSomethingToFinish = Boolean(persistence.ideaId) || persistence.hasMeaningfulAnswer;
	const canFinish =
		currConversation.length >= minConversationLenForCreateAndEnd &&
		!result &&
		hasSomethingToFinish;

	/** a link whose navigation was held back by a failed save: clicking
	 * it again leaves without the unsaved changes */
	const leaveAnywayHrefRef = useRef<string | null>(null);

	// Client-side navigation (Astro's router) unloads this page's state:
	// a pending autosave would be dropped (the pagehide flush is
	// fire-and-forget). Hold any same-app navigation while anything is
	// unsaved, wait for the writes to land, then run the router's own
	// loader. When the save can't land, say so and let a second attempt
	// leave anyway.
	useEffect(() => {
		const onBeforePreparation = (event: Event) => {
			const routerEvent = event as BeforePreparationEvent;
			if (!hasUnsavedChanges()) return;
			if (leaveAnywayHrefRef.current === routerEvent.to) {
				leaveAnywayHrefRef.current = null;
				return;
			}
			// take the navigation out of the router's hands; either run it
			// once the saves land, or explain and stay
			event.preventDefault();
			void (async () => {
				try {
					await whenIdle();
				} catch {
					leaveAnywayHrefRef.current = routerEvent.to;
					setChatError({
						message:
							"Your latest changes are not saved yet. Retry saving, or click the link again to leave without them.",
						source: "save"
					});
					return;
				}
				await routerEvent.loader();
			})();
		};
		document.addEventListener("astro:before-preparation", onBeforePreparation);
		return () => document.removeEventListener("astro:before-preparation", onBeforePreparation);
		// the two members the interceptor reads, not the whole object: a
		// broader key would re-register this listener on every streamed
		// token (saveState changes while a reply streams)
	}, [hasUnsavedChanges, whenIdle]);

	// after the draft loader: its status decides whether input waits
	const restoringDraft = draftStatus === "loading";
	const loadingContinuation = chatType === CHAT_TYPE.CONTINUATION && !parentIdea;
	const micBlockedReason = loadingContinuation
		? "Loading the original idea…"
		: dailyStatusLoading
			? "Loading today's status…"
			: restoringDraft
				? "Restoring your draft…"
				: llmBlocked
					? "Set up AI to begin"
					: sttBlocked
						? "Speech model needed"
						: undefined;
	const sendBlockedReason = loadingContinuation
		? "Loading the original idea…"
		: dailyStatusLoading
			? "Loading today's status…"
			: restoringDraft
				? "Restoring your draft…"
				: llmBlocked
					? "Set up AI to reply"
					: undefined;

	/** Wrap up the session and generate its summary. The summary needs
	 * the idea to exist, so a session without one is created first; the
	 * effect below finishes the job when the create lands (the later
	 * unrelated-create guard lives in the ref, see finishAfterCreateRef).
	 * Both finish buttons route through here, so a failed create retries
	 * instead of dead-ending "Generate Summary" on a session without an
	 * idea id. */
	const finishSession = useCallback(() => {
		// nothing to save or summarize yet: the create waits for a real
		// answer, so wrapping up now would spin forever
		if (!hasSomethingToFinish) return;
		if (sessionIdeaId) {
			handleGetResult();
			return;
		}
		finishAfterCreateRef.current = true;
		setIsWrappingUp(true);
		requestCreate();
	}, [sessionIdeaId, handleGetResult, requestCreate, hasSomethingToFinish]);

	// the keyboard twin of the finish buttons (Cmd/Ctrl+Enter)
	useEffect(() => {
		if (!canFinish) return;
		const onKeyDown = (event: KeyboardEvent): void => {
			if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
				event.preventDefault();
				finishSession();
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, [canFinish, finishSession]);

	useEffect(() => {
		if (!finishAfterCreateRef.current || !persistence.ideaId) return;
		finishAfterCreateRef.current = false;
		handleGetResult();
	}, [persistence.ideaId, handleGetResult]);

	if (result) {
		return (
			<FinishedResultSection
				ideaId={persistence.ideaId}
				result={result}
				readyForFinish={session.readyToSave}
				isComplete={session.resultComplete}
				saveError={session.resultSaveError}
				onRetrySave={session.retrySaveResult}
				droppedTurns={session.resultTruncated ? session.resultDroppedTurns : 0}
				generationStopped={session.resultStoppedEarly}
				onRetryGeneration={session.handleGetResult}
				onKeepDraft={() => session.keepPartialResult(result)}
				statusText={session.resultStatus}
				cardsMissing={session.resultCardsMissing}
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
				title="Idea not found"
				paragraphs={[
					chatType === CHAT_TYPE.CONTINUATION
						? "The idea you wanted to continue no longer exists in this library."
						: "The idea you were giving feedback on no longer exists in this library."
				]}
			/>
		);
	} else {
		const aiMessageContent = session.isUserResendRequired
			? session.inappropriateUserTranscript
			: (session.streamingReply ?? findMostRecentAssistantContent(currConversation));
		const enableSkip =
			currConversation.length > 1 && session.conversationState === CONVERSATION_STATE.Idle;
		// canReviseLastAnswer guarantees the answer sits right before the
		// final assistant reply
		const lastUserContent = session.canReviseLastAnswer
			? currConversation[currConversation.length - 2]!.content
			: null;
		const feedbackParent = chatType === CHAT_TYPE.FEEDBACK ? parentIdea : undefined;
		const mainSectionChildren = [
			<AssistantResponseText
				key="assistant-response"
				styleSetting={feedbackParent && "feedback"}
				content={aiMessageContent}
				enableSkip={enableSkip}
				handleSkipQuestion={session.askADifferentQuestion}
				didFailToSend={session.isUserResendRequired}
			/>,
			lastUserContent !== null ? (
				<LastAnswerActions
					key="last-answer-actions"
					lastAnswer={lastUserContent}
					onEdit={(text) => void session.editLastUserMessage(text)}
					onRemove={session.removeLastUserTurn}
				/>
			) : null,
			<ChatRecorder
				key="chat-recorder"
				isCompressed={Boolean(feedbackParent)}
				conversationState={session.conversationState}
				dispatch={dispatch}
				currConversation={currConversation}
				setCurrConversation={setCurrConversation}
				setSaveState={persistence.setSaveState}
				handleGetResponse={session.handleGetResponse}
				modelLoading={modelAvailability === "loading"}
				micBlockedReason={micBlockedReason}
				sendBlockedReason={sendBlockedReason}
			/>
		];

		return (
			<section>
				{restoringDraft && (
					<p role="status" className="text-sm text-stone-500 text-center pt-4">
						Restoring your draft…
					</p>
				)}
				{chatError && (
					<ChatErrorBanner
						aiError={chatError.message}
						source={chatError.source}
						onDismiss={() => setChatError(null)}
						onRetry={
							chatError.source === "save" ? persistence.retryAutosave : undefined
						}
					/>
				)}
				<ModelStatusNotice
					availability={modelAvailability}
					error={aiStatus.llm.error}
					progress={aiStatus.llm.progress}
				/>
				<ChatTopBar
					parentIdea={feedbackParent}
					showTranscript={showTranscript}
					setShowTranscript={setShowTranscript}
					leftButtonIcon={
						fromGuideParam && !getQueryParam(QUERY_PARAMS.id)
							? "arrow-back-outline"
							: null
					}
					leftButtonHref="/get-started"
					saveState={persistence.saveState}
				/>
				{feedbackParent ? (
					<ChatFeedbackMainSection
						showTranscript={showTranscript}
						setShowTranscript={setShowTranscript}
						parentIdea={feedbackParent}
						currConversation={currConversation}
						conversationState={session.conversationState}
						handleGetResult={finishSession}
						handleEarlyFinish={finishSession}
						isWrappingUp={isWrappingUp}
						minConversationLenForEnd={minConversationLenForCreateAndEnd}
						canFinish={hasSomethingToFinish}
					>
						{mainSectionChildren}
					</ChatFeedbackMainSection>
				) : (
					<ChatIdeaMainSection
						showTranscript={showTranscript}
						setShowTranscript={setShowTranscript}
						currConversation={currConversation}
						conversationState={session.conversationState}
						handleGetResult={finishSession}
						handleEarlyFinish={finishSession}
						isWrappingUp={isWrappingUp}
						minConversationLenForEnd={minConversationLenForCreateAndEnd}
						canFinish={hasSomethingToFinish}
					>
						{mainSectionChildren}
					</ChatIdeaMainSection>
				)}
			</section>
		);
	}
}
