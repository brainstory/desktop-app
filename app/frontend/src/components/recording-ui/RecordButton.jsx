import { useState, useEffect, useRef, useContext } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AppContext } from "@src/components/chat/reusable/AppWrapper";
import { CONVERSATION_STATE } from "../../const";
import { ICON } from "./RecordIcons";
import { transcribeApi } from "@helpers/api/ai";
import ChangeInputTypeButton from "./ChangeInputTypeButton";

function RecordButton({
	isRecording,
	isDisabledOverride,
	conversationState,
	setIsRecording,
	setIsTranscribing,
	setTranscript,
	getCoachResponse,
	time,
	resetTimer,
	isCompressed
}) {
	const RECORDING_MAX_DURATION = 240000; // 4 minutes

	const [warningType, setWarningType] = useState(null);
	const [readyToSend, setReadyToSend] = useState(
		conversationState === CONVERSATION_STATE.ReadyToSendUserTranscript
	);
	const [status, setStatus] = useState("idle");
	const [isTextInput, setIsTextInput] = useState(false);
	const [userTextInput, setUserTextInput] = useState("");
	const [micPermissionDenied, setMicPermissionDenied] = useState(false);
	const [micStarting, setMicStarting] = useState(false);
	const [errorMessage, setErrorMessage] = useState(null);
	const context = useContext(AppContext);

	// Refs so the unmount cleanup always sees the live values.
	const timerRef = useRef(null);
	const activeRecordingRef = useRef(false);

	const buttonSizing = isCompressed ? "w-[54px] h-[54px]" : "w-[96px] h-[96px]";
	const shadownSizing = isCompressed
		? "w-[60px] h-[60px] group-hover:w-[66px] group-hover:h-[66px]"
		: "w-[108px] h-[108px] group-hover:w-[116px] group-hover:h-[116px]";

	useEffect(() => {
		setReadyToSend(conversationState === CONVERSATION_STATE.ReadyToSendUserTranscript);
	}, [conversationState]);

	useEffect(() => {
		if (readyToSend === true) {
			resetTimer();
			getCoachResponse()
				.catch((err) => console.error("coach response failed", err))
				.finally(() => {
					setReadyToSend(false);
					// clear any possible user text input
					setUserTextInput("");
				});
		}
	}, [readyToSend]);

	// If the component goes away mid-recording (user ends the chat, page
	// navigates), stop the timer and release the Rust-side microphone.
	useEffect(() => {
		return () => {
			if (timerRef.current) {
				clearTimeout(timerRef.current);
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

	const stopWavCapture = async () => {
		activeRecordingRef.current = false;
		const wav = await invoke("stop_voice_capture"); // ArrayBuffer
		if (!wav || wav.byteLength <= 44) {
			handleError("no audio captured");
			return;
		}
		generateTranscript(new Blob([wav], { type: "audio/wav" }));
	};

	const handleToggleRecording = async () => {
		if (isRecording) {
			setIsRecording(false);
			setWarningType(null);
			clearTimeout(timerRef.current);
			setStatus("idle");
			try {
				await stopWavCapture();
			} catch (error) {
				handleError(error);
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
				const recordingTimeout = setTimeout(() => {
					stopWavCapture().catch((error) => handleError(error));
					setIsRecording(false);
					setWarningType("timer"); // Set warning when time limit is exceeded
				}, RECORDING_MAX_DURATION);
				timerRef.current = recordingTimeout;
				setStatus("recording");
			} catch (error) {
				console.error("Error accessing microphone:", error);
				setIsRecording(false);
				setStatus("idle");
				setErrorMessage(String(error?.message ?? error));
				setMicPermissionDenied(true);
			} finally {
				setMicStarting(false);
			}
		}
	};

	function handleError(error) {
		setIsTranscribing(false);
		setTranscript("");
		console.log(error);
		setReadyToSend(false);
		setErrorMessage(String(error?.message ?? error));
		setWarningType("error");
	}

	function generateTranscript(blobby, retriesLeft = 1) {
		setIsTranscribing(true);

		const apiCall = transcribeApi;

		apiCall(blobby)
			.then((transcript) => {
				setTranscript(transcript);
				setIsTranscribing(false);
				// No setReadyToSend here: sending is driven by
				// conversationState turning ReadyToSendUserTranscript, which
				// the parent sets only after the user message is appended.
			})
			.catch((error) => {
				if (retriesLeft >= 1) {
					retriesLeft--;
					setTimeout(() => {
						console.log("Error in API, retrying once");
						generateTranscript(blobby, retriesLeft);
					}, 500);
				} else {
					handleError(error);
				}
			});
	}

	function handleTextareaChange(event) {
		setUserTextInput(event.target.value);
	}

	function handleTextSend() {
		setTranscript(userTextInput);
	}

	function renderInputComponent() {
		if (isTextInput) {
			return (
				<div className="flex justify-center items-end w-full">
					<textarea
						className="w-full max-w-[500px] text-sm px-4 py-2 border border-stone-200 rounded-md focus:outline-none focus:border-blue-500 resize-none md:resize-y"
						placeholder="Type something..."
						aria-label="Type your response"
						rows="5"
						value={userTextInput}
						onChange={handleTextareaChange}
						onKeyDown={handleKeyDown}
					></textarea>
					<button
						className="flex items-center h-[36px] w-[36px] ml-2 p-2 rounded-full bg-blue-500 text-white hover:bg-blue-600 focus:outline-none focus:ring focus:border-blue-300"
						aria-label="Send message"
						onClick={handleTextSend}
						onKeyDown={handleKeyDown}
					>
						<ion-icon class="w-8 h-8 hydrated" name="send" aria-hidden="true"></ion-icon>
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
			let disabled =
				micStarting ||
				isDisabledOverride ||
				conversationState === CONVERSATION_STATE.WaitingForCoach ||
				conversationState === CONVERSATION_STATE.TranscribingUser;
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
					<p>{status === "idle" ? "ready to listen" : status}</p>
				</div>
			);
		}
	}

	function onInterfaceToggle() {
		setIsTextInput(!isTextInput);
	}

	function handleKeyDown(event) {
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
				<div className={warningBoxStyle}>
					<p className="text-sm font-semibold flex-1">
						{warningType === "error"
							? (errorMessage ??
								"An error occurred while transcribing. Please try again.")
							: "Maximum recording time 4 minutes reached. Send message to continue."}
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
			{/* show time limit almost up warning if 20 seconds from max  */}
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
