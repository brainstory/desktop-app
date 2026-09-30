import { useState, useEffect, useRef } from "react";
import { useStore } from "@nanostores/react";
import { $aiStatus, llmBusy } from "@components/global/aiStatusStore";
import {
	CONVERSATION_STATE,
	CHAT_SAVE_STATE,
	MIN_CONVERSATION_LENGTH_BEFORE_SAVE,
	CHAT_TYPE,
	ASK_A_DIFFERENT_QUESTION
} from "@src/const";
import {
	handleStreamResult,
	addConversationMessage,
	removeLastConversationMessage,
	findMostRecentAssistantContent,
	getFirstPrompt,
	useIdeaIdFromUrl
} from "@helpers/chat";
import { markGettingStartedDone } from "@helpers/storage";
import { getIdeaApi, createIdeaApi, updateIdeaApi } from "@helpers/api/idea";
import { generateResponseApi, generateResponseStreamApi } from "@helpers/api/ai";
import {
	getQueryParam,
	callApiWithRetry,
	normalizeApiError,
	isModerationError
} from "@helpers/helpers";

import ChatRecorder from "@components/chat/reusable/ChatRecorder";
import FinishedResultSection from "@components/chat/reusable/FinishedResultSection";
import ChatIdeaMainSection from "@components/chat/ChatIdeaMainSection";
import ChatFeedbackMainSection from "@components/chat/ChatFeedbackMainSection";
import ErrorSection from "@components/error/ErrorSection";

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
	const [conversationState, setConversationState] = useState(CONVERSATION_STATE.Start);
	const [ideaId, setIdeaId] = useState<string | undefined>(draftId);
	/** true if result finished generating result */
	const [readyToSave, setReadyToSave] = useState(false);
	/** true if there's no id in query param and conversation meets length */
	/** true if user message was inappropriate by the AI provider */
	const [isUserResendRequired, setIsUserResendRequired] = useState(false);
	/** if isUserResendRequired is true, then this field value is the inappropriate flagged transcript */
	const [inappropriateUserTranscript, setInappropriateUserTranscript] = useState<string | null>(
		null
	);
	const [showTranscript, setShowTranscript] = useState(false);
	/** display error component as the section instead of mic ui */
	const [errorComponent, setErrorComponent] = useState<React.ReactNode>();
	/** true if saving is in progress, false if already saved */
	const [saveState, setSaveState] = useState(CHAT_SAVE_STATE.WAITING);
	/** error from the AI layer that is not the 469 resend case (e.g. no model downloaded) */
	const [aiError, setAiError] = useState<string | null>(null);

	const aiStatus = useStore($aiStatus);
	const showModelLoading = llmBusy(aiStatus);
	const firstPrompt = getFirstPrompt(chatType);
	const [currConversation, setCurrConversation] = useState([
		{ role: "assistant", content: firstPrompt }
	]);
	const minConversationLenForCreateAndEnd =
		MIN_CONVERSATION_LENGTH_BEFORE_SAVE[chatType] ||
		MIN_CONVERSATION_LENGTH_BEFORE_SAVE.DEFAULT;

	const fetchParentIdea = (parentId: string): void => {
		getIdeaApi(parentId)
			.then((res) => {
				const ideaContent = {
					id: res.id,
					title: res.title,
					summary: res.summary
				};
				document.title = `Feedback for "${res.title}"`;
				setParentIdea(ideaContent);
			})
			.catch((err) => {
				console.log("Parent Idea not found with ID", parentId, err);
				setErrorComponent(
					<ErrorSection
						title="Shared idea not found"
						paragraphs={["This idea does not exist or you do not have access"]}
					/>
				);
			});
	};

	const hasMounted = useRef(false);
	/** guards createIdeaApi: the effect can legally re-run while a create
	 *  is still in flight (StrictMode double-invoke, conversation updates);
	 *  a second create would produce a duplicate idea row */
	const creatingIdeaRef = useRef(false);
	/** latest conversation length without re-running the mount fetch */
	const conversationLengthRef = useRef(currConversation.length);
	useEffect(() => {
		conversationLengthRef.current = currConversation.length;
	}, [currConversation.length]);
	/** monotonic autosave sequence: only the newest save may settle state */
	const autosaveSeqRef = useRef(0);
	/** parent idea already fetched (id keyed) */
	const fetchedParentRef = useRef<string | null>(null);

	useIdeaIdFromUrl(hasMounted, setIdeaId);

	// derived: an idea must be created as soon as the conversation is long
	// enough and no idea row exists yet
	const readyToCreateIdea =
		!ideaId && currConversation.length >= minConversationLenForCreateAndEnd;

	useEffect(() => {
		// this useEffect is to protect from creating duplicates of the same idea if the currConversation is set twice
		// perhaps to the same value but the useEffect is triggered since array variables are pointers to memory
		if (readyToCreateIdea && !creatingIdeaRef.current) {
			creatingIdeaRef.current = true;
			createIdeaApi(result, currConversation, chatType, parentIdParam, dailyLogId)
				.then((createdIdeaId) => {
					creatingIdeaRef.current = false;
					setIdeaId(createdIdeaId);
					const url = new URL(window.location.href);
					const params = new URLSearchParams(url.search);
					params.set("id", createdIdeaId);
					history.pushState(null, "", "?" + params.toString());
				})
				.catch((err) => {
					creatingIdeaRef.current = false;
					setAiError(`Could not save this session: ${normalizeApiError(err)}`);
					// readyToCreateIdea stays true; the next conversation
					// update re-runs this effect and retries creation
				});
		}
	}, [readyToCreateIdea, currConversation, dailyLogId, result, chatType, parentIdParam]);

	// Autosave: debounced (rapid user/assistant turns must not fire one
	// write each), sequenced (a stale completion can never overwrite the
	// top-bar state of a newer save), and the SAVING state lives here so
	// the append sites don't have to set it.
	useEffect(() => {
		if (!ideaId || currConversation.length < minConversationLenForCreateAndEnd) {
			return;
		}
		const seq = ++autosaveSeqRef.current;
		const timer = window.setTimeout(() => {
			setSaveState(CHAT_SAVE_STATE.SAVING);
			updateIdeaApi(ideaId, currConversation)
				.then(() => {
					if (seq !== autosaveSeqRef.current) return;
					// don't show SAVED visual for saving the user message so that
					// the switch from SAVING to SAVED doesn't happen twice
					if (currConversation[currConversation.length - 1].role === "assistant") {
						setSaveState(CHAT_SAVE_STATE.SUCCESS);
					}
				})
				.catch((e) => {
					if (seq !== autosaveSeqRef.current) return;
					setSaveState(CHAT_SAVE_STATE.FAILED);
					setAiError(`Autosave failed: ${normalizeApiError(e)}`);
				});
		}, 400);
		return () => window.clearTimeout(timer);
	}, [currConversation, ideaId, minConversationLenForCreateAndEnd]);

	// Draft load: runs once per ideaId (NOT on every message - the old
	// currConversation.length dependency re-fetched mid-session and made
	// two in-flight reads able to resolve out of order).
	useEffect(() => {
		if (ideaId) {
			getIdeaApi(ideaId)
				.then((res) => {
					if (res.summary) {
						// if result already present, change to idea result page
						// it's not the smoothest transition I'll admit
						window.location.href = `/idea?id=${res.id}`;
						return;
					}
					const savedConversation = [...(res.transcript ?? [])];
					// Only adopt the saved transcript if it has more messages than
					// what we hold locally (via the ref: this closure sees the
					// mount-time conversation): restores a resumed draft, but
					// never clobbers newer messages with a stale fetch.
					if (savedConversation.length > conversationLengthRef.current) {
						setCurrConversation(savedConversation);
						const lastMessage = savedConversation.at(-1);
						if (lastMessage?.role === "user") {
							setConversationState(CONVERSATION_STATE.ReadyToSendUserTranscript);
						} else {
							setConversationState(CONVERSATION_STATE.Idle);
						}
					}

					const parentIdData = res.parentIdea?.id;
					if (parentIdData && fetchedParentRef.current !== parentIdData) {
						fetchedParentRef.current = parentIdData;
						fetchParentIdea(parentIdData);
					}
				})
				.catch((err) => {
					console.log("idea not found with ID", ideaId, err);
					setErrorComponent(
						<ErrorSection
							title="Draft idea not found"
							paragraphs={[
								"This draft does not exist or you do not have access",
								"If you did not mean to open a draft, start a new idea instead"
							]}
						/>
					);
				});
		} else if (parentIdParam && fetchedParentRef.current !== parentIdParam) {
			// when idea id isn't in the query parameter bc the idea hasn't been created yet
			fetchedParentRef.current = parentIdParam;
			fetchParentIdea(parentIdParam);
		}
	}, [ideaId, parentIdParam]);

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
						setAiError(normalizeApiError(err));
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
		// idea creation can still be in flight for young conversations; never
		// generate a result we can't save
		if (!ideaId) {
			setAiError("Still saving this session - try again in a moment.");
			return;
		}
		conversationEndCallbacks();
		setConversationState(CONVERSATION_STATE.FinishWithResult);
		const resultFinishedCallbacks = async (
			result: string,
			structuredResult: unknown
		): Promise<void> => {
			// final update with saving result - only claim success once it saved
			try {
				await updateIdeaApi(ideaId, currConversation, result, structuredResult);
				setReadyToSave(true);
			} catch (e) {
				setAiError(`Could not save your summary: ${normalizeApiError(e)}`);
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
				setAiError(normalizeApiError(err));
				// re-enable the button so the user can retry
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

	if (result) {
		return (
			<FinishedResultSection
				ideaId={ideaId}
				result={result}
				parentSummary={parentIdea?.summary}
				readyForFinish={readyToSave}
			/>
		);
	} else if (errorComponent) {
		return errorComponent;
	} else {
		const aiMessageContent = isUserResendRequired
			? inappropriateUserTranscript
			: findMostRecentAssistantContent(currConversation);
		const enableSkip =
			currConversation.length > 1 && conversationState === CONVERSATION_STATE.Idle;
		const mainSectionChildren = [
			<AssistantResponseText
				key="assistant-response"
				styleSetting={parentIdea && "feedback"}
				content={aiMessageContent}
				enableSkip={enableSkip}
				handleSkipQuestion={askADifferentQuestion}
				didFailToSend={isUserResendRequired}
			/>,
			<ChatRecorder
				key="chat-recorder"
				isCompressed={Boolean(parentIdea)}
				conversationState={conversationState}
				setConversationState={setConversationState}
				currConversation={currConversation}
				setCurrConversation={setCurrConversation}
				setSaveState={setSaveState}
				handleGetResponse={() => Promise.resolve(handleGetResponse())}
			/>
		];

		return (
			<section className="wow">
				{aiError && (
					<div
						role="alert"
						className="flex items-center justify-between gap-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 m-4 text-sm"
					>
						<span>
							<b>Hmm, the AI couldn&rsquo;t respond:</b> {aiError}
						</span>
						<span className="flex gap-2 shrink-0">
							<a
								className="underline font-semibold whitespace-nowrap"
								href="/profile?tab=aiModels"
							>
								Open AI settings
							</a>
							<button
								className="underline text-stone-500 whitespace-nowrap"
								onClick={() => setAiError(null)}
							>
								Dismiss
							</button>
						</span>
					</div>
				)}

				{showModelLoading && (
					<p
						role="status"
						className="mx-4 mt-2 text-sm text-stone-500 bg-stone-100 rounded-lg px-4 py-2"
					>
						Loading the AI model - you can type already, sending unlocks when it is
						ready...
					</p>
				)}
				<ChatTopBar
					parentIdea={parentIdea}
					showTranscript={showTranscript}
					setShowTranscript={setShowTranscript}
					leftButtonIcon={
						fromGuideParam && !getQueryParam("id") ? "arrow-back-outline" : null
					}
					leftButtonHref="/get-started"
					saveState={saveState}
				/>
				{parentIdea ? (
					<ChatFeedbackMainSection
						showTranscript={showTranscript}
						setShowTranscript={setShowTranscript}
						parentIdea={parentIdea}
						currConversation={currConversation}
						conversationState={conversationState}
						handleGetResult={handleGetResult}
						minConversationLenForEnd={minConversationLenForCreateAndEnd}
					>
						{mainSectionChildren}
					</ChatFeedbackMainSection>
				) : (
					<ChatIdeaMainSection
						showTranscript={showTranscript}
						setShowTranscript={setShowTranscript}
						currConversation={currConversation}
						conversationState={conversationState}
						handleGetResult={handleGetResult}
						minConversationLenForEnd={minConversationLenForCreateAndEnd}
					>
						{mainSectionChildren}
					</ChatIdeaMainSection>
				)}
			</section>
		);
	}
}
