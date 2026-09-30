import { CONVERSATION_STATE, CHAT_SAVE_STATE } from "@src/const";
import { addConversationMessage } from "@helpers/chat";
import AudioRecorder from "@components/recording-ui/AudioRecorder";
import type { ChatMessage } from "@src/types";
import type { Dispatch, SetStateAction } from "react";
import { useCallback } from "react";

/**
 * PLEASE: state var conversationState setting logic should only be in this component!
 *
 * aka don't pass setConversationState as a prop to children
 * This is so state changes can tracked easier rather than having it done in child components
 */
interface ChatRecorderProps {
	conversationState: string;
	setConversationState: (state: string) => void;
	currConversation: ChatMessage[];
	setCurrConversation: Dispatch<SetStateAction<ChatMessage[]>>;
	setSaveState: (state: string) => void;
	handleGetResponse: () => void | Promise<void>;
	isCompressed?: boolean | string | null;
}

export default function ChatRecorder({
	conversationState,
	setConversationState,
	currConversation,
	setCurrConversation,
	setSaveState,
	handleGetResponse,
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
				setConversationState(CONVERSATION_STATE.TranscribingUser);
			} else {
				// transcription failed - release the UI back to idle
				setConversationState(CONVERSATION_STATE.Idle);
			}
		},
		[setConversationState]
	);

	const handleTranscript = useCallback(
		async (userMessage: string) => {
			const isUser = true;
			// only after the user message is actually appended does
			// sending become safe (the coach must see it); the SAVING
			// save-state is owned by ChatSection's autosave effect
			await addConversationMessage(
				userMessage,
				isUser,
				currConversation,
				setCurrConversation
			);
			setConversationState(CONVERSATION_STATE.ReadyToSendUserTranscript);
		},
		[currConversation, setCurrConversation, setConversationState]
	);

	const handleCoachResponse = useCallback(async () => {
		await handleGetResponse();
	}, [handleGetResponse]);

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
				conversationState={conversationState}
				currConversation={currConversation}
				setIsTranscribing={handleIsTranscribing}
				onTranscript={handleTranscript}
				getCoachResponse={handleCoachResponse}
				startRecordingCallback={handleStartRecording}
			/>
		</section>
	);
}
