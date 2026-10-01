import { CHAT_SAVE_STATE } from "@src/const";
import { addConversationMessage } from "@helpers/chat";
import AudioRecorder from "@components/recording-ui/AudioRecorder";
import type { ChatMessage } from "@src/types";
import type { ChatSessionEvent, CoachResponseOutcome } from "@components/chat/useChatSession";
import type { Dispatch, SetStateAction } from "react";
import { useCallback } from "react";

/**
 * Conversation state changes are session events (useChatSession's
 * reducer); the recorder dispatches the recording/transcription ones and
 * doesn't pass dispatch further down.
 */
interface ChatRecorderProps {
	conversationState: string;
	dispatch: (event: ChatSessionEvent) => void;
	currConversation: ChatMessage[];
	setCurrConversation: Dispatch<SetStateAction<ChatMessage[]>>;
	setSaveState: (state: string) => void;
	handleGetResponse: () => Promise<CoachResponseOutcome>;
	/** the language model is still loading: no recording or sending yet */
	modelLoading?: boolean;
	isCompressed?: boolean | string | null;
}

export default function ChatRecorder({
	conversationState,
	dispatch,
	currConversation,
	setCurrConversation,
	setSaveState,
	handleGetResponse,
	modelLoading = false,
	isCompressed
}: ChatRecorderProps) {
	/** if the max content is met, disable further addition to conversation */
	// unbounded conversations degrade model quality; force the result.
	// Derived during render: the conversation only ever grows, so once this
	// is true it stays true.
	const forceFinish = currConversation.length > 100;

	// Stable identities: AudioRecorder/RecordButton keep these in effect
	// dependencies, and inline arrows would re-run the effects on every
	// render of this section.
	const handleStartRecording = useCallback(() => {
		setSaveState(CHAT_SAVE_STATE.WAITING);
	}, [setSaveState]);

	const handleIsTranscribing = useCallback(
		(isTranscribing: boolean) => {
			if (isTranscribing) {
				dispatch({ type: "transcriptionStarted" });
			} else {
				// transcription failed - release the UI back to idle
				dispatch({ type: "transcriptionFailed" });
			}
		},
		[dispatch]
	);

	const handleTranscript = useCallback(
		(userMessage: string) => {
			const isUser = true;
			// Resending the message that failed or was cancelled (it is
			// still the unanswered last message) only re-asks the coach
			// instead of adding a duplicate.
			const last = currConversation[currConversation.length - 1];
			const isResend = last?.role === "user" && last.content === userMessage;
			// only after the user message is actually appended does
			// sending become safe (the coach must see it); the SAVING
			// save-state is owned by ChatSection's autosave effect
			if (!isResend) {
				addConversationMessage(userMessage, isUser, currConversation, setCurrConversation);
			}
			dispatch({ type: "userMessageReady" });
		},
		[currConversation, setCurrConversation, dispatch]
	);

	const handleCoachResponse = useCallback(() => handleGetResponse(), [handleGetResponse]);

	return (
		<section
			className={`w-full ${
				isCompressed ? "md:py-4 py-2" : "md:p-8 p-4"
			} bg-white flex justify-center items-center flex-col rounded-lg`}
		>
			{forceFinish && (
				<p role="status" className="text-sm text-stone-500 mb-2">
					This conversation reached its length limit - wrap it up with the summary button.
				</p>
			)}
			<AudioRecorder
				isCompressed={isCompressed}
				isDisabled={forceFinish}
				modelLoading={modelLoading}
				conversationState={conversationState}
				setIsTranscribing={handleIsTranscribing}
				onTranscript={handleTranscript}
				getCoachResponse={handleCoachResponse}
				startRecordingCallback={handleStartRecording}
			/>
		</section>
	);
}
