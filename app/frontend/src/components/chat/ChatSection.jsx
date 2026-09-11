import { useState, useEffect, useRef } from "react";
import {
	CONVERSATION_STATE,
	CHAT_SAVE_STATE,
	ERROR_MESSAGE_MAP,
	MIN_CONVERSATION_LENGTH_BEFORE_SAVE,
	CHAT_TYPE
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
import { getQueryParam, callApiWithRetry, normalizeApiError } from "@helpers/helpers";

import ChatRecorder from "@components/chat/reusable/ChatRecorder";
import FinishedResultSection from "@components/chat/reusable/FinishedResultSection";
import ChatIdeaMainSection from "@components/chat/ChatIdeaMainSection";
import ChatFeedbackMainSection from "@components/chat/ChatFeedbackMainSection";
import ErrorSection from "@components/error/ErrorSection";

import { AssistantResponseText } from "@components/chat/AssistantResponseText";
import ChatTopBar from "./reusable/ChatTopBar";

const parentId = getQueryParam("parentId");
const isFromGuide = getQueryParam("topic");
let chatType = CHAT_TYPE.ORIGINAL;
if (getQueryParam("dailyIntent") === "true") {
	chatType = CHAT_TYPE.DAILY_INTENT;
} else if (parentId) {
	chatType = CHAT_TYPE.FEEDBACK;
}

export function ChatSection({ draftId, dailyLogId, conversationEndCallbacks }) {
	const [result, setResult] = useState("");
	const [parentIdea, setParentIdea] = useState();
	const [conversationState, setConversationState] = useState(CONVERSATION_STATE.Start);
	const [ideaId, setIdeaId] = useState(draftId);
	/** true if result finished generating result */
	const [readyToSave, setReadyToSave] = useState(false);
	/** true if there's no id in query param and conversation meets length */
	const [readyToCreateIdea, setReadyToCreateIdea] = useState(false);
	/** true if user message was inappropriate by the AI provider */
	const [isUserResendRequired, setIsUserResendRequired] = useState(false);
	/** if isUserResendRequired is true, then this field value is the inappropriate flagged transcript */
	const [inappropriateUserTranscript, setInappropriateUserTranscript] = useState(null);
	const [showTranscript, setShowTranscript] = useState(false);
	/** display error component as the section instead of mic ui */
	const [errorComponent, setErrorComponent] = useState();
	/** true if saving is in progress, false if already saved */
	const [saveState, setSaveState] = useState(CHAT_SAVE_STATE.WAITING);
	/** error from the AI layer that is not the 469 resend case (e.g. no model downloaded) */
	const [aiError, setAiError] = useState(null);

	const firstPrompt = getFirstPrompt(chatType);
	const [currConversation, setCurrConversation] = useState([
		{ role: "assistant", content: firstPrompt }
	]);
	const askADifferentQuestionString = "Ask me a different question!";
	const minConversationLenForCreateAndEnd =
		MIN_CONVERSATION_LENGTH_BEFORE_SAVE[chatType] ||
		MIN_CONVERSATION_LENGTH_BEFORE_SAVE.DEFAULT;

	const hasMounted = useRef(false);

	useIdeaIdFromUrl(hasMounted, setIdeaId);

	useEffect(() => {
		// this useEffect is to protect from creating duplicates of the same idea if the currConversation is set twice
		// perhaps to the same value but the useEffect is triggered since array variables are pointers to memory
		// WARNING: this is a bandiad as it doesn't help debug why currConversation might be set twice to the same value
		if (readyToCreateIdea) {
			createIdeaApi(result, currConversation, chatType, parentId, dailyLogId)
				.then((createdIdeaId) => {
					setIdeaId(createdIdeaId);
					let url = new URL(window.location.href);
					let params = new URLSearchParams(url.search);
					params.set("id", createdIdeaId);
					history.pushState(null, null, "?" + params.toString());
				})
				.catch((err) => {
					setAiError(`Could not save this session: ${normalizeApiError(err)}`);
					// allow the next conversation update to retry creation
					setReadyToCreateIdea(false);
				});
		}
	}, [readyToCreateIdea]);

	useEffect(() => {
		if (currConversation.length >= minConversationLenForCreateAndEnd) {
			if (ideaId) {
				updateIdeaApi(ideaId, currConversation)
					.then(() => {
						// don't show SAVED visual for saving the user message so that
						// the switch from SAVING to SAVED doesn't happen twice
						if (currConversation[currConversation.length - 1].role === "assistant") {
							setSaveState(CHAT_SAVE_STATE.SUCCESS);
						}
					})
					.catch((e) => {
						setSaveState(CHAT_SAVE_STATE.FAILED);
						setAiError(`Autosave failed: ${normalizeApiError(e)}`);
					});
			} else {
				setReadyToCreateIdea(true);
			}
		}
	}, [currConversation]);

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
					const savedConversation = [...res.transcript];
					// Only adopt the saved transcript if it has more messages than
					// what we hold locally: restores a resumed draft, but never
					// clobbers newer messages with a stale fetch (which made
					// messages visibly vanish mid-session).
					if (savedConversation.length > currConversation.length) {
						setCurrConversation(savedConversation);
						const lastMessage = savedConversation.at(-1);
						if (lastMessage?.role === "user") {
							setConversationState(CONVERSATION_STATE.ReadyToSendUserTranscript);
						} else {
							setConversationState(CONVERSATION_STATE.Idle);
						}
					}

					const parentIdData = res.parentIdea?.id;
					if (parentIdData) {
						fetchParentIdea(parentIdData);
					}
				})
				.catch((err) => {
					console.log("idea not found with ID " + ideaId, err);
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
		} else if (parentId) {
			// when idea id isn't in the query parameter bc the idea hasn't been created yet
			fetchParentIdea(parentId);
		}
	}, [ideaId]);

	const fetchParentIdea = (parentId) => {
		getIdeaApi(parentId)
			.then((res) => {
				let ideaContent = {
					id: res.id,
					title: res.title,
					summary: res.summary
				};
				document.title = `Feedback for \"${res.title}\"`;
				setParentIdea(ideaContent);
			})
			.catch((err) => {
				console.log("Parent Idea not found with ID " + parentId, err);
				setErrorComponent(
					<ErrorSection
						title="Shared idea not found"
						paragraphs={["This idea does not exist or you do not have access"]}
					/>
				);
			});
	};

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
					addConversationMessage(
						message,
						isUser,
						currConversation,
						setCurrConversation,
						(newLength) =>
							newLength >= minConversationLenForCreateAndEnd &&
							setSaveState(CHAT_SAVE_STATE.SAVING)
					);
					setIsUserResendRequired(false);
					setInappropriateUserTranscript(null);
				})
				.catch((err) => {
					const message = normalizeApiError(err);
					if (message.includes(ERROR_MESSAGE_MAP[469])) {
						setIsUserResendRequired(true);
						const removedMessage = removeLastConversationMessage(
							currConversation,
							setCurrConversation
						);
						setInappropriateUserTranscript(removedMessage);
					} else {
						setAiError(message);
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
		const resultFinishedCallbacks = async (result, structuredResult) => {
			// final update with saving result - only claim success once it saved
			try {
				await updateIdeaApi(ideaId, currConversation, result, structuredResult);
				setReadyToSave(true);
			} catch (e) {
				setAiError(`Could not save your summary: ${normalizeApiError(e)}`);
				setConversationState(CONVERSATION_STATE.Idle);
				return;
			}
			if (isFromGuide) {
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
			askADifferentQuestionString,
			isUser,
			currConversation,
			setCurrConversation,
			(newLength) => {
				newLength >= minConversationLenForCreateAndEnd &&
					setSaveState(CHAT_SAVE_STATE.SAVING);
			}
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
				allowFinishMinConversationLength={minConversationLenForCreateAndEnd}
				isCompressed={parentIdea && "feedback"}
				conversationState={conversationState}
				setConversationState={setConversationState}
				currConversation={currConversation}
				setCurrConversation={setCurrConversation}
				setSaveState={(state) =>
					currConversation.length >= minConversationLenForCreateAndEnd &&
					setSaveState(state)
				}
				handleGetResponse={handleGetResponse}
			/>
		];

		return (
			<section className="wow">
				{aiError && (
					<div className="flex items-center justify-between gap-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 m-4 text-sm">
						<span>
							<b>Hmm, the AI couldn't respond:</b> {aiError}
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

				<ChatTopBar
					parentIdea={parentIdea}
					showTranscript={showTranscript}
					setShowTranscript={setShowTranscript}
					leftButtonIcon={
						isFromGuide && !getQueryParam("id") ? "arrow-back-outline" : null
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
