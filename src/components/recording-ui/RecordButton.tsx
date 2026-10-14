import { useState, useEffect, useRef, useContext } from "react";
import { AppContext } from "@src/components/chat/reusable/AppWrapper";
import { CONVERSATION_STATE, type ConversationState } from "@src/const";
import { transcribeApi, cancelTranscriptionApi } from "@helpers/api/ai";
import { callApiWithRetry } from "@helpers/helpers";
import { useUnmountSignal } from "@src/hooks/useUnmountSignal";
import { describeError } from "@helpers/describeError";
import { getInputMode, setInputMode } from "@helpers/storage";
import ChangeInputTypeButton from "./ChangeInputTypeButton";
import { useVoiceCapture } from "./useVoiceCapture";
import { isTranscriptionCancelled, isTransientTranscriptionError } from "./transcription";
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
	/** non-undefined: the mic is unusable (e.g. no speech model), reason shown */
	micBlockedReason?: string;
	/** non-undefined: typed sends would only fail, reason shown */
	sendBlockedReason?: string;
	conversationState: ConversationState;
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
	micBlockedReason,
	sendBlockedReason,
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
	// remembered across sessions; text is the sane default when the mic
	// cannot be used at all
	const [isTextInput, setIsTextInput] = useState(
		() => getInputMode() === "text" || Boolean(micBlockedReason)
	);
	const [userTextInput, setUserTextInput] = useState("");
	const context = useContext(AppContext);
	const { warningType, setWarningType, errorMessage, setErrorMessage, dismiss } =
		useRecordingWarnings();
	/** text of the visually hidden live region (screen readers only; the
	 * visible recording timer is aria-live="off" so it doesn't chatter) */
	const [announcement, setAnnouncement] = useState("");
	/** local mirror of the parent's transcribing flag: this component needs
	 * it to offer the cancel affordance while a transcription runs */
	const [isTranscribing, setIsTranscribingLocal] = useState(false);
	const updateTranscribing = (value: boolean): void => {
		setIsTranscribingLocal(value);
		setIsTranscribing(value);
	};

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
	const unmountSignal = useUnmountSignal();

	const handleError = (error: unknown): void => {
		updateTranscribing(false);
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
		updateTranscribing(false);
		setErrorMessage(
			"Couldn't stop the microphone - try again. If it stays on, restart the app."
		);
		setWarningType("error");
	};

	const generateTranscript = (blobby: Blob): void => {
		updateTranscribing(true);
		setAnnouncement(TRANSCRIBING);
		// Transient transport hiccups deserve one retry; configuration
		// and input problems ("no model downloaded", bad audio) never fix
		// themselves. Unmounting cancels a pending retry.
		callApiWithRetry(
			() => transcribeApi(blobby),
			1,
			isTransientTranscriptionError,
			unmountSignal()
		)
			.then((transcript) => {
				if (!mountedRef.current) return;
				setAnnouncement("");
				// silence or noise: there is nothing to send, and an empty
				// user message would only confuse the coach
				if (!transcript.trim()) {
					updateTranscribing(false);
					setErrorMessage("No speech was heard. Try again, or type your message.");
					setWarningType("error");
					return;
				}
				onTranscript(transcript);
				updateTranscribing(false);
			})
			.catch((error: unknown) => {
				if (!mountedRef.current) return;
				// a user cancel: the UI already returned to idle, and an
				// error banner for a button the person pressed would be
				// noise
				if (isTranscriptionCancelled(error)) {
					updateTranscribing(false);
					setAnnouncement("");
					return;
				}
				handleError(error);
			});
	};

	/** Stop waiting for a transcription. The backend poll cancels an
	 * external request at once and discards a local inference's result
	 * when it finishes. */
	const handleCancelTranscription = (): void => {
		cancelTranscriptionApi().catch((err) =>
			console.error("failed to cancel the transcription", err)
		);
		// release the UI now; the in-flight invoke settles with the
		// cancellation error handled above
		updateTranscribing(false);
		setAnnouncement("");
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

	// Keyboard: Space toggles the mic with the same guards as the click
	// (and only when no field has focus - typing a space must stay
	// typing); Escape cancels a transcription. The mic button itself
	// stays focusable, so this is a shortcut, not the only path.
	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent): void => {
			const target = event.target as HTMLElement | null;
			const tag = target?.tagName?.toLowerCase();
			const inField =
				tag === "input" ||
				tag === "textarea" ||
				tag === "select" ||
				target?.isContentEditable === true;
			if (event.key === "Escape") {
				if (isTranscribing && !inField) {
					event.preventDefault();
					handleCancelTranscription();
				}
				return;
			}
			if (event.key !== " " || inField) return;
			const micUsable =
				!isDisabledOverride &&
				!modelLoading &&
				!micBlockedReason &&
				!voice.micStarting &&
				!voice.micPermissionDenied &&
				conversationState !== CONVERSATION_STATE.WaitingForCoach &&
				conversationState !== CONVERSATION_STATE.TranscribingUser;
			if (micUsable) {
				event.preventDefault();
				void voice.toggleRecording();
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
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
		if (!trimmed || isDisabledOverride || modelLoading || sendBlockedReason) return;
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
					sendLockedReason={
						sendBlockedReason ?? (modelLoading ? "Loading model…" : undefined)
					}
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
					isDisabled={
						Boolean(isDisabledOverride) ||
						(modelLoading && !isRecording) ||
						(Boolean(micBlockedReason) && !isRecording)
					}
					micStarting={voice.micStarting}
					conversationState={conversationState}
					elapsedSeconds={time}
					status={
						micBlockedReason && !isRecording
							? micBlockedReason
							: modelLoading && !isRecording
								? "Loading model…"
								: voice.status
					}
					onToggle={() => void voice.toggleRecording()}
					onAnimationTrigger={context.setSludgeman}
					isCompressed={isCompressed}
				/>
			)}
			{isTranscribing && (
				<div className="text-center mt-1">
					<button
						type="button"
						className="text-xs text-stone-500 underline hover:no-underline"
						onClick={handleCancelTranscription}
					>
						Cancel transcription
					</button>
				</div>
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
				onToggle={() => {
					const next = !isTextInput;
					setIsTextInput(next);
					setInputMode(next ? "text" : "voice");
				}}
			/>
		</>
	);
}

export default RecordButton;
