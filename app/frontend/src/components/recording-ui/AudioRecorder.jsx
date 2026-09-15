import { useState, useEffect } from "react";
import { CONVERSATION_STATE } from "../../const";
import RecordButton from "./RecordButton";

import ChatStateNotification from "@components/chat/reusable/ChatStateNotification";

function useTimer(isRunning, setTime) {
	useEffect(() => {
		let interval = null;

		if (isRunning) {
			interval = setInterval(() => {
				setTime((prevTime) => prevTime + 1);
			}, 1000);
		} else {
			clearInterval(interval);
		}

		return () => clearInterval(interval);
	}, [isRunning]);
}

const AudioRecorder = ({
	isDisabled,
	conversationState,
	getCoachResponse,
	setUiTranscript,
	setIsTranscribing,
	isCompressed,
	startRecordingCallback
}) => {
	const [time, setTime] = useState(0);
	const [isRunning, setIsRunning] = useState(false);
	const [transcript, setTranscript] = useState("");

	// Timer
	useTimer(isRunning, setTime);

	useEffect(() => {
		if (transcript.length > 0) {
			setUiTranscript(transcript);
			// Clear the transcript so saying or typing the same thing twice
			// in a row still triggers the effect (identical strings would
			// otherwise be swallowed by React's state dedup).
			setTranscript("");
		}
	}, [transcript]);

	useEffect(() => {
		if (isRunning) {
			startRecordingCallback();
		}
	}, [isRunning]);

	return (
		<div className="w-full text-center">
			<div key="state-renderer w-full">
				<ChatStateNotification conversationState={conversationState} />
			</div>
			<div
				className={`${
					(conversationState === CONVERSATION_STATE.WaitingForCoach ||
						conversationState === CONVERSATION_STATE.TranscribingUser) &&
					"hidden"
				} ${
					isCompressed ? "gap-3" : "gap-8"
				} text-sm text-stone-600 flex justify-between flex-col items-center mb-2`}
			>
				<RecordButton
					isCompressed={isCompressed}
					isRecording={isRunning}
					isDisabledOverride={isDisabled}
					conversationState={conversationState}
					setIsRecording={setIsRunning}
					setIsTranscribing={setIsTranscribing}
					setTranscript={setTranscript}
					getCoachResponse={getCoachResponse}
					time={time}
					resetTimer={() => setTime(0)}
				/>
			</div>
		</div>
	);
};

export default AudioRecorder;
