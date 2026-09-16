import type { ChatMessage } from "@src/types";
import { useState, useEffect } from "react";
import { CONVERSATION_STATE } from "../../const";
import RecordButton from "./RecordButton";

import ChatStateNotification from "@components/chat/reusable/ChatStateNotification";
import type { Dispatch, SetStateAction } from "react";

function useTimer(isRunning: boolean, setTime: Dispatch<SetStateAction<number>>) {
	useEffect(() => {
		let interval: number | null = null;

		if (isRunning) {
			interval = window.setInterval(() => {
				setTime((prevTime) => prevTime + 1);
			}, 1000);
		} else {
			if (interval !== null) window.clearInterval(interval);
		}

		return () => {
			if (interval !== null) window.clearInterval(interval);
		};
	}, [isRunning, setTime]);
}

interface AudioRecorderProps {
	isDisabled?: boolean;
	currConversation?: ChatMessage[];
	conversationState: string;
	getCoachResponse: () => Promise<void>;
	onTranscript: (transcript: string) => void;
	setIsTranscribing: (transcribing: boolean) => void;
	isCompressed?: boolean | string | null;
	startRecordingCallback?: () => void;
}

const AudioRecorder = ({
	isDisabled,
	conversationState,
	getCoachResponse,
	onTranscript,
	setIsTranscribing,
	isCompressed,
	startRecordingCallback
}: AudioRecorderProps) => {
	const [time, setTime] = useState(0);
	const [isRunning, setIsRunning] = useState(false);


	// Timer
	useTimer(isRunning, setTime);

	useEffect(() => {
		if (isRunning) {
			startRecordingCallback?.();
		}
	}, [isRunning, startRecordingCallback]);

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
					onTranscript={onTranscript}
					getCoachResponse={getCoachResponse}
					time={time}
					resetTimer={() => setTime(0)}
				/>
			</div>
		</div>
	);
};

export default AudioRecorder;
