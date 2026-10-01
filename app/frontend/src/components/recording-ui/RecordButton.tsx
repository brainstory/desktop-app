import { useState, useEffect, useRef, useContext } from "react";
import { AppContext } from "@src/components/chat/reusable/AppWrapper";
import { CONVERSATION_STATE } from "../../const";
import { transcribeApi } from "@helpers/api/ai";
import { callApiWithRetry, normalizeApiError } from "@helpers/helpers";
import { describeError } from "@helpers/describeError";
import ChangeInputTypeButton from "./ChangeInputTypeButton";
import { useVoiceCapture } from "./useVoiceCapture";
import { MicButton, MicPermissionDenied } from "./MicButton";
import { RecordingWarnings, TextComposer, useRecordingWarnings } from "./RecordingWarnings";

interface RecordButtonProps {
	isRecording: boolean;
	isDisabledOverride?: boolean;
	conversationState: string;
	setIsRecording: (recording: boolean) => void;
	setIsTranscribing: (transcribing: boolean) => void;
	onTranscript: (transcript: string) => void;
	getCoachResponse: () => Promise<void>;
	time: number;
	resetTimer: () => void;
	isCompressed?: boolean | string | null;
}

function RecordButton({
	isRecording,
	isDisabledOverride,
	conversationState,
	setIsRecording,
	setIsTranscribing,
	onTranscript,
	getCoachResponse,
	time,
	resetTimer,
	isCompressed
}: RecordButtonProps) {
	const readyToSend = conversationState === CONVERSATION_STATE.ReadyToSendUserTranscript;
	const [isTextInput, setIsTextInput] = useState(false);
	const [userTextInput, setUserTextInput] = useState("");
	const context = useContext(AppContext);
	const { warningType, setWarningType, errorMessage, setErrorMessage, dismiss } =
		useRecordingWarnings();

	// one-shot guard so the coach response fires once per ReadyToSend
	const respondedRef = useRef(false);

	const handleError = (error: unknown): void => {
		setIsTranscribing(false);
		console.error("recording/transcription failed", error);
		const described = describeError(error);
		setErrorMessage(
			described.action ? `${described.message} ${described.action}` : described.message
		);
		setWarningType("error");
	};

	const handleStopError = (error: unknown): void => {
		console.error("failed to stop the microphone", error);
		setIsTranscribing(false);
		setErrorMessage(
			"Couldn't stop the microphone - try again. If it stays on, restart the app."
		);
		setWarningType("error");
	};

	const generateTranscript = (blobby: Blob): void => {
		setIsTranscribing(true);
		// Transient transport hiccups deserve one retry; configuration
		// problems ("no model downloaded") never fix themselves.
		const isTransient = (error: unknown): boolean =>
			!normalizeApiError(error).includes("not downloaded yet");
		callApiWithRetry(() => transcribeApi(blobby), 1, isTransient)
			.then((transcript) => {
				onTranscript(transcript);
				setIsTranscribing(false);
			})
			.catch(handleError);
	};

	const voice = useVoiceCapture({
		isRecording,
		setIsRecording,
		onWavCaptured: generateTranscript,
		onError: handleError,
		onStopError: handleStopError,
		onTimeLimit: () => setWarningType("timer")
	});

	useEffect(() => {
		if (readyToSend && !respondedRef.current) {
			respondedRef.current = true;
			resetTimer();
			getCoachResponse()
				.then(() => {
					// the message made it into the conversation - clear the
					// composer for the next one. On failure the text stays
					// so the user can edit and resend it.
					setUserTextInput("");
				})
				.catch((err) => console.error("coach response failed", err));
		}
		if (!readyToSend) {
			respondedRef.current = false;
		}
	}, [readyToSend, getCoachResponse, resetTimer]);

	function handleTextSend() {
		const trimmed = userTextInput.trim();
		if (!trimmed) return;
		onTranscript(trimmed);
	}

	return (
		<>
			{isTextInput ? (
				<TextComposer
					value={userTextInput}
					onChange={setUserTextInput}
					onSend={handleTextSend}
				/>
			) : voice.micPermissionDenied ? (
				<MicPermissionDenied
					errorMessage={errorMessage}
					onRetry={voice.retryAfterPermissionError}
					isCompressed={isCompressed}
				/>
			) : (
				<MicButton
					isRecording={isRecording}
					isDisabled={Boolean(isDisabledOverride)}
					micStarting={voice.micStarting}
					conversationState={conversationState}
					elapsedSeconds={time}
					status={voice.status}
					onToggle={() => void voice.toggleRecording()}
					onAnimationTrigger={context.setSludgeman}
					isCompressed={isCompressed}
				/>
			)}
			<RecordingWarnings
				warningType={warningType}
				errorMessage={errorMessage}
				onDismiss={dismiss}
				secondsRemaining={voice.secondsRemaining - time}
				isRecording={isRecording}
			/>
			<ChangeInputTypeButton
				isTextInput={isTextInput}
				onToggle={() => setIsTextInput(!isTextInput)}
			/>
		</>
	);
}

export default RecordButton;
