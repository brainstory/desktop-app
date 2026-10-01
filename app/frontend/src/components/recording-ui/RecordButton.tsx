import { useState, useEffect, useRef, useContext } from "react";
import { AppContext } from "@src/components/chat/reusable/AppWrapper";
import { CONVERSATION_STATE } from "../../const";
import { transcribeApi } from "@helpers/api/ai";
import { callApiWithRetry } from "@helpers/helpers";
import { describeError } from "@helpers/describeError";
import ChangeInputTypeButton from "./ChangeInputTypeButton";
import { useVoiceCapture } from "./useVoiceCapture";
import { isTransientTranscriptionError } from "./transcription";
import { MicButton, MicPermissionDenied } from "./MicButton";
import { RecordingWarnings, TextComposer, useRecordingWarnings } from "./RecordingWarnings";
import type { CoachResponseOutcome } from "@components/chat/useChatSession";

const TRANSCRIBING = "Transcribing…";

interface RecordButtonProps {
	isRecording: boolean;
	isDisabledOverride?: boolean;
	/** the language model is still loading: recording and sending would
	 * only fail, typing is fine */
	modelLoading?: boolean;
	conversationState: string;
	setIsRecording: (recording: boolean) => void;
	setIsTranscribing: (transcribing: boolean) => void;
	onTranscript: (transcript: string) => void;
	getCoachResponse: () => Promise<CoachResponseOutcome>;
	time: number;
	resetTimer: () => void;
	isCompressed?: boolean | string | null;
}

function RecordButton({
	isRecording,
	isDisabledOverride,
	modelLoading = false,
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
	/** text of the visually hidden live region (screen readers only; the
	 * visible recording timer is aria-live="off" so it doesn't chatter) */
	const [announcement, setAnnouncement] = useState("");

	// one-shot guard so the coach response fires once per ReadyToSend
	const respondedRef = useRef(false);
	// a transcription (or its retry) can settle after the chat is gone:
	// ignore it then instead of feeding a conversation nobody sees
	const mountedRef = useRef(true);
	useEffect(() => {
		mountedRef.current = true;
		return () => {
			mountedRef.current = false;
		};
	}, []);

	const handleError = (error: unknown): void => {
		setIsTranscribing(false);
		// the error itself is announced by the role="alert" banner; only
		// retract a stale "Transcribing…"
		setAnnouncement((current) => (current === TRANSCRIBING ? "" : current));
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
		setAnnouncement(TRANSCRIBING);
		// Transient transport hiccups deserve one retry; configuration
		// and input problems ("no model downloaded", bad audio) never fix
		// themselves. No retry once unmounted either.
		callApiWithRetry(
			() => transcribeApi(blobby),
			1,
			(error) => mountedRef.current && isTransientTranscriptionError(error)
		)
			.then((transcript) => {
				if (!mountedRef.current) return;
				setAnnouncement("");
				onTranscript(transcript);
				setIsTranscribing(false);
			})
			.catch((error: unknown) => {
				if (!mountedRef.current) return;
				handleError(error);
			});
	};

	const voice = useVoiceCapture({
		isRecording,
		setIsRecording,
		onWavCaptured: generateTranscript,
		onError: handleError,
		onStopError: handleStopError,
		onTimeLimit: () => setWarningType("timer"),
		onRecordingChange: (change) =>
			setAnnouncement(change === "started" ? "Recording started" : "Recording stopped")
	});

	useEffect(() => {
		if (readyToSend && !respondedRef.current) {
			respondedRef.current = true;
			resetTimer();
			getCoachResponse()
				.then((outcome) => {
					// the coach answered - clear the composer for the next
					// message. Otherwise (flagged, failed, cancelled) the
					// text stays so the user can edit and resend it.
					if (outcome === "sent") {
						setUserTextInput("");
					}
				})
				.catch((err) => console.error("coach response failed", err));
		}
		if (!readyToSend) {
			respondedRef.current = false;
		}
	}, [readyToSend, getCoachResponse, resetTimer]);

	function handleTextSend() {
		const trimmed = userTextInput.trim();
		// never overlap generations: one message at a time
		if (!trimmed || isDisabledOverride || modelLoading) return;
		onTranscript(trimmed);
	}

	return (
		<>
			<p className="sr-only" role="status" aria-live="polite">
				{announcement}
			</p>
			{isTextInput ? (
				<TextComposer
					value={userTextInput}
					onChange={setUserTextInput}
					onSend={handleTextSend}
					disabled={Boolean(isDisabledOverride)}
					sendLockedReason={modelLoading ? "Loading model…" : undefined}
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
					isDisabled={Boolean(isDisabledOverride) || (modelLoading && !isRecording)}
					micStarting={voice.micStarting}
					conversationState={conversationState}
					elapsedSeconds={time}
					status={modelLoading && !isRecording ? "Loading model…" : voice.status}
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
