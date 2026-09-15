import { useState, useEffect } from "react";
import { CONVERSATION_STATE, CHAT_SAVE_STATE } from "@src/const";
import { addConversationMessage } from "@helpers/chat";
import AudioRecorder from "@components/recording-ui/AudioRecorder";

/**
 * PLEASE: state var conversationState setting logic should only be in this component!
 *
 * aka don't pass setConversationState as a prop to children
 * This is so state changes can tracked easier rather than having it done in child components
 */
export default function ChatRecorder({
	conversationState,
	setConversationState,
	currConversation,
	setCurrConversation,
	setSaveState,
	handleGetResponse,
	isCompressed,
	allowFinishMinConversationLength
}) {
	/** if the max content is met, disable further addition to conversation */
	const [forceFinish, setForceFinish] = useState(currConversation.length > 100);

	useEffect(() => {
		if (currConversation.length > 100) {
			// unbounded conversations degrade model quality; force the result
			setForceFinish(true);
		}
	}, [currConversation]);

	return (
		<section
			className={`w-full ${
				isCompressed ? "md:py-4 py-2" : "md:p-8 p-4"
			} bg-white flex justify-center items-center flex-col rounded-lg`}
		>
			<AudioRecorder
				isCompressed={isCompressed}
				isDisabled={forceFinish}
				conversationState={conversationState}
				currConversation={currConversation}
				setIsTranscribing={(isTranscribing) => {
					if (isTranscribing) {
						setConversationState(CONVERSATION_STATE.TranscribingUser);
					} else {
						// transcription failed - release the UI back to idle
						setConversationState(CONVERSATION_STATE.Idle);
					}
				}}
			setUiTranscript={async (userMessage) => {
				const isUser = true;
				const next = await addConversationMessage(
					userMessage,
					isUser,
					currConversation,
					setCurrConversation
				);
				// only after the user message is actually appended does
				// sending become safe (the coach must see it)
				if (next.length >= (allowFinishMinConversationLength ?? 0)) {
					setSaveState(CHAT_SAVE_STATE.SAVING);
				}
				setConversationState(CONVERSATION_STATE.ReadyToSendUserTranscript);
			}}
				getCoachResponse={async () => await handleGetResponse()}
				startRecordingCallback={() => setSaveState(CHAT_SAVE_STATE.WAITING)}
			/>
		</section>
	);
}
