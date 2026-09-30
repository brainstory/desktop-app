import { useState, useEffect, useRef, useContext } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AppContext } from "@src/components/chat/reusable/AppWrapper";
import { CONVERSATION_STATE } from "../../const";
import { ICON } from "./RecordIcons";
import { transcribeApi } from "@helpers/api/ai";
import { callApiWithRetry, normalizeApiError } from "@helpers/helpers";
import { describeError } from "@helpers/describeError";
import ChangeInputTypeButton from "./ChangeInputTypeButton";

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
	const RECORDING_MAX_DURATION = 240000; // 4 minutes

	/** mm:ss readout for the live recording timer */
	function formatElapsed(totalSeconds: number): string {
		const minutes = Math.floor(totalSeconds / 60);
		const seconds = totalSeconds % 60;
		return `${minutes}:${String(seconds).padStart(2, "0")}`;
	}

	const [warningType, setWarningType] = useState<string | null>(null);
	// derived: the parent drives when the coach should respond
	const readyToSend = conversationState === CONVERSATION_STATE.ReadyToSendUserTranscript;
	const [status, setStatus] = useState("idle");
	const [isTextInput, setIsTextInput] = useState(false);
	const [userTextInput, setUserTextInput] = useState("");
	const [micPermissionDenied, setMicPermissionDenied] = useState(false);
	const [micStarting, setMicStarting] = useState(false);
	const [errorMessage, setErrorMessage] = useState<string | null>(null);
	const context = useContext(AppContext);

	// Refs so the unmount cleanup always sees the live values.
	const timerRef = useRef<number | null>(null);
	const activeRecordingRef = useRef(false);

	const buttonSizing = isCompressed ? "w-[54px] h-[54px]" : "w-[96px] h-[96px]";
	const shadownSizing = isCompressed
		? "w-[60px] h-[60px] group-hover:w-[66px] group-hover:h-[66px]"
		: "w-[108px] h-[108px] group-hover:w-[116px] group-hover:h-[116px]";

	// one-shot guard so the coach response fires once per ReadyToSend
	const respondedRef = useRef(false);

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

	// If the component goes away mid-recording (user ends the chat, page
	// navigates), stop the timer and release the Rust-side microphone.
	useEffect(() => {
		return () => {
			if (timerRef.current) {
				if (timerRef.current !== null) clearTimeout(timerRef.current);
			}
			if (activeRecordingRef.current) {
				invoke("stop_voice_capture").catch(() => {});
			}
		};
	}, []);

	// ---- Audio capture happens in the Rust process (cpal): the webview's
	// getUserMedia delivers silent audio in some permission states. ----

	const startWavCapture = async () => {
		await invoke("start_voice_capture");
	};

	const stopWavCapture = async (): Promise<void> => {
		const wav = await invoke<ArrayBuffer>("stop_voice_capture"); // ArrayBuffer
		// Only clear the flag once the Rust side has actually stopped:
		// clearing first would make the unmount cleanup skip the stop and
		// leave cpal capturing forever.
		activeRecordingRef.current = false;
		if (!wav || wav.byteLength <= 44) {
			handleError("no audio captured");
			return;
		}
		generateTranscript(new Blob([wav], { type: "audio/wav" }));
	};

	const handleToggleRecording = async (): Promise<void> => {
		if (isRecording) {
			setWarningType(null);
			if (timerRef.current !== null) clearTimeout(timerRef.current);
			setStatus("stopping mic...");
			try {
				await stopWavCapture();
				setIsRecording(false);
				setStatus("idle");
			} catch (error) {
				// The stop failed: the mic may still be running, so keep
				// the recording state (the button offers to stop again and
				// the unmount cleanup still fires) and say what happened.
				handleMicStopError(error);
			}
		} else {
			setStatus("starting mic...");
			setWarningType(null);
			// flip immediately - the first start in a session can take a
			// moment (CoreAudio device init), and the button should respond
			// instantly; reverted if the mic can't start
			setIsRecording(true);
			setMicStarting(true);
			try {
				await startWavCapture();
				activeRecordingRef.current = true;
				const recordingTimeout = window.setTimeout(() => {
					setWarningType("timer"); // Set warning when time limit is exceeded
					stopWavCapture()
						.then(() => setIsRecording(false))
						.catch((error) => handleMicStopError(error));
				}, RECORDING_MAX_DURATION);
				timerRef.current = recordingTimeout;
				setStatus("recording");
			} catch (error) {
				console.error("Error accessing microphone:", error);
				setIsRecording(false);
				setStatus("idle");
				setErrorMessage(normalizeApiError(error));
				setMicPermissionDenied(true);
			} finally {
				setMicStarting(false);
			}
		}
	};

	function handleMicStopError(error: unknown): void {
		console.error("failed to stop the microphone", error);
		setIsTranscribing(false);
		setErrorMessage(
			"Couldn't stop the microphone - try again. If it stays on, restart the app."
		);
		setWarningType("error");
		setStatus("recording");
	}

	function handleError(error: unknown): void {
		setIsTranscribing(false);
		console.error("recording/transcription failed", error);
		const described = describeError(error);
		setErrorMessage(
			described.action ? `${described.message} ${described.action}` : described.message
		);
		setWarningType("error");
	}

	function generateTranscript(blobby: Blob): void {
		setIsTranscribing(true);

		// Transient transport hiccups deserve one retry; configuration
		// problems ("no model downloaded") never fix themselves, so retrying
		// just doubles the wait before the real error shows.
		const isTransient = (error: unknown): boolean => {
			const message = normalizeApiError(error);
			return !message.includes("not downloaded yet");
		};

		callApiWithRetry(() => transcribeApi(blobby), 1, isTransient)
			.then((transcript) => {
				onTranscript(transcript);
				setIsTranscribing(false);
				// No setReadyToSend here: sending is driven by
				// conversationState turning ReadyToSendUserTranscript, which
				// the parent sets only after the user message is appended.
			})
			.catch((error) => {
				handleError(error);
			});
	}

	function handleTextareaChange(event: React.ChangeEvent<HTMLTextAreaElement>): void {
		setUserTextInput(event.target.value);
	}

	function handleTextSend() {
		const trimmed = userTextInput.trim();
		if (!trimmed) return;
		onTranscript(trimmed);
	}

	const enterHint = (
		<p className="w-full max-w-[500px] text-xs text-stone-500 text-left mt-1">
			Enter to send &middot; Shift+Enter for a new line
		</p>
	);

	function renderInputComponent() {
		if (isTextInput) {
			return (
				<div className="flex justify-center items-end w-full">
					<div className="flex flex-col w-full max-w-[500px]">
						<textarea
							className="w-full text-sm px-4 py-2 border border-stone-200 rounded-md focus:outline-none focus:border-accent-500 resize-none md:resize-y"
							placeholder="Type something..."
							aria-label="Type your response"
							rows={5}
							value={userTextInput}
							onChange={handleTextareaChange}
							onKeyDown={handleKeyDown}
						></textarea>
						{enterHint}
					</div>
					<button
						className="flex items-center h-[36px] w-[36px] ml-2 p-2 rounded-full bg-accent-600 text-white hover:bg-accent-700 focus:outline-none focus:ring focus:border-accent-400 disabled:opacity-40 disabled:cursor-not-allowed"
						aria-label="Send message"
						disabled={!userTextInput.trim()}
						onClick={handleTextSend}
					>
						<ion-icon
							class="w-8 h-8 hydrated"
							name="send"
							aria-hidden="true"
						></ion-icon>
					</button>
				</div>
			);
		} else if (micPermissionDenied) {
			return (
				<div>
					<div className="w-[244px] mx-auto group relative flex justify-center items-center my-4">
						<div
							className={`${shadownSizing} pointer-events-none rounded-full absolute bg-gradient-to-r from-pink-500 via-pink-400 to-pink-700 opacity-40 blur transition-all duration-300`}
						></div>
						<button
							className={`${buttonSizing} bg-white relative flex justify-center items-center shadow-xl border-[1px] border-stone-200 text-stone-500 font-bold rounded-full group:hover:scale-105 transform transition-transform hover:bg-stone-100 transition-colors duration-300`}
							onClick={() => {
								// let the user retry (transient errors like a
								// busy mic land here too, not just permission)
								setMicPermissionDenied(false);
								handleToggleRecording();
							}}
						>
							{ICON.MicOff}
						</button>
					</div>
					<div className="text-black text-base">
						<p>Couldn&rsquo;t start the microphone.</p>
						<p>
							{errorMessage ? (
								errorMessage
							) : (
								<>
									<b>Allow microphone</b> in your system settings and try again
								</>
							)}
						</p>
					</div>
				</div>
			);
		} else {
			const { setSludgeman } = context;
			let icon;
			const baseStyleClasses = `${buttonSizing} focus-visible:ring-4 focus-visible:outline-none focus-visible:ring-pink-300 relative flex items-center justify-center bg-white shadow-xl border-[1px] border-stone-200 text-stone-500 hover:text-stone-700 font-bold rounded-full disabled:opacity-40 disabled:cursor-not-allowed group:hover:scale-105 transform transition-transform hover:bg-stone-100 transition-colors duration-300 `;
			const disabled =
				micStarting ||
				isDisabledOverride ||
				conversationState === CONVERSATION_STATE.WaitingForCoach ||
				conversationState === CONVERSATION_STATE.TranscribingUser;
			const micAriaLabel = micStarting
				? "Starting microphone"
				: isRecording
					? "Stop recording"
					: "Start recording";
			let onClickHandler = () => {};

			if (micStarting) {
				onClickHandler = () => {};
			} else if (isRecording) {
				icon = ICON.Recording;
				onClickHandler = () => {
					setSludgeman("idle");
					handleToggleRecording();
				};
			} else {
				icon = ICON.ReadyToRecord;
				onClickHandler = () => {
					setSludgeman("jump");
					handleToggleRecording();
				};
			}
			return (
				<div>
					<div className="w-[244px] group relative flex justify-center items-center my-4">
						<div
							className={`${
								isRecording && "animate-spin"
							} ${shadownSizing} pointer-events-none group-hover:w-[116px] group-hover:h-[116px] rounded-full absolute bg-gradient-to-r from-pink-500 via-pink-400 to-pink-700 opacity-60 hover:opacity-90 blur transition-all duration-300`}
						></div>
						<button
							disabled={disabled}
							aria-label={micAriaLabel}
							className={baseStyleClasses}
							onClick={onClickHandler}
						>
							{micStarting ? (
								<div
									className="w-9 h-9 rounded-full border-[3px] border-stone-200 border-t-pink-500 animate-spin"
									role="status"
									aria-label="starting microphone"
								></div>
							) : (
								icon
							)}
						</button>
					</div>
					{isRecording ? (
						<p className="text-sm text-stone-600 tabular-nums" aria-live="off">
							<span className="inline-block w-2 h-2 mr-2 rounded-full bg-red-500 align-middle"></span>
							recording &middot; {formatElapsed(time)}
						</p>
					) : (
						<p>{status === "idle" ? "ready to listen" : status}</p>
					)}
				</div>
			);
		}
	}

	function onInterfaceToggle() {
		setIsTextInput(!isTextInput);
	}

	function handleKeyDown(event: React.KeyboardEvent): void {
		// Enter sends; Shift+Enter inserts a newline (the field is a
		// multi-line textarea, so users expect both)
		if (event.key === "Enter" && !event.shiftKey) {
			event.preventDefault();
			handleTextSend();
		}
	}

	const warningBoxStyle =
		"flex justify-center fixed z-50 left-0 right-0 w-3/4 mx-auto bg-red-700 text-white py-4 px-4 rounded-lg shadow-lg flex items-center top-8";

	const secondsRemaining = RECORDING_MAX_DURATION / 1000 - time;

	return (
		<>
			{renderInputComponent()}
			{warningType && (
				<div role="alert" className={warningBoxStyle}>
					<p className="text-sm font-semibold flex-1">
						{warningType === "error"
							? (errorMessage ??
								"An error occurred while transcribing. Please try again.")
							: "You hit the 4-minute limit for one recording. It's being transcribed now — keep going in your next message."}
					</p>
					<button
						className="text-white/80 hover:text-white text-lg font-bold px-2 shrink-0"
						aria-label="dismiss message"
						onClick={() => {
							setWarningType(null);
							setErrorMessage(null);
						}}
					>
						×
					</button>
				</div>
			)}
			<ChangeInputTypeButton isTextInput={isTextInput} onToggle={onInterfaceToggle} />
			{/* show the almost-up warning in the last 30 seconds of the max recording time */}
			{isRecording && secondsRemaining <= 30 && secondsRemaining > 0 && (
				<div className="flex justify-center fixed z-50 top-8 left-0 right-0 w-3/4 mx-auto bg-amber-400 text-black py-4 px-4 rounded-lg shadow-lg flex items-center">
					<p className="text-sm font-semibold">
						{`Max recording limit of 4 minutes almost reached. You have ${secondsRemaining} seconds remaining`}
					</p>
				</div>
			)}
		</>
	);
}

export default RecordButton;
