import { useState, useEffect, useCallback } from "react";
import { CONVERSATION_STATE } from "../../const";
import RecordButton from "./RecordButton";
import { cn } from "@helpers/cn";
import type { CoachResponseOutcome } from "@components/chat/useChatSession";

import ChatStateNotification from "@components/chat/reusable/ChatStateNotification";
import type { Dispatch, SetStateAction } from "react";
import type { ConversationState } from "@src/const";

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
	/** the language model is still loading: mic and Send wait for it */
	modelLoading?: boolean;
	/** non-undefined: the mic is unusable, with this reason shown */
	micBlockedReason?: string;
	/** non-undefined: typed sends are locked, with this reason shown */
	sendBlockedReason?: string;
	conversationState: ConversationState;
	getCoachResponse: () => Promise<CoachResponseOutcome>;
	onTranscript: (transcript: string) => void;
	setIsTranscribing: (transcribing: boolean) => void;
	isCompressed?: boolean | string | null;
	startRecordingCallback?: () => void;
}

const AudioRecorder = ({
	isDisabled,
	modelLoading = false,
	micBlockedReason,
	sendBlockedReason,
	conversationState,
	getCoachResponse,
	onTranscript,
	setIsTranscribing,
	isCompressed,
	startRecordingCallback
}: AudioRecorderProps) => {
	const [time, setTime] = useState(0);
	const [isRunning, setIsRunningState] = useState(false);

	// stable identity: RecordButton keeps this in effect dependencies
	const resetTimer = useCallback(() => setTime(0), []);

	/** Every recording starts from zero. Reset here, in the start
	 * handler, rather than in an effect: after a failed, empty or
	 * cancelled transcription nobody else resets the readout, and the
	 * 4-minute warning would fire early (or go negative) on the next
	 * attempt. */
	const setIsRunning = useCallback(
		(running: boolean) => {
			if (running) setTime(0);
			setIsRunningState(running);
		},
		[]
	);

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
				className={cn(
					isCompressed ? "gap-3" : "gap-8",
					"text-sm text-stone-600 flex justify-between flex-col items-center mb-2"
				)}
			>
				<RecordButton
					isCompressed={isCompressed}
					isRecording={isRunning}
					// while waiting on the coach or a transcription the mic
					// stays rendered but disabled - display:none dropped
					// focus to <body>
					isDisabledOverride={
						isDisabled ||
						conversationState === CONVERSATION_STATE.WaitingForCoach ||
						conversationState === CONVERSATION_STATE.TranscribingUser
					}
					modelLoading={modelLoading}
					micBlockedReason={micBlockedReason}
					sendBlockedReason={sendBlockedReason}
					conversationState={conversationState}
					setIsRecording={setIsRunning}
					setIsTranscribing={setIsTranscribing}
					onTranscript={onTranscript}
					getCoachResponse={getCoachResponse}
					time={time}
					resetTimer={resetTimer}
				/>
			</div>
		</div>
	);
};

export default AudioRecorder;
