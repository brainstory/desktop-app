import { useCallback, useEffect, useRef, useState } from "react";
import { COMMANDS } from "@src/tauri/commands";
import { invoke } from "@tauri-apps/api/core";

/**
 * Rust-side voice capture lifecycle: start/stop cpal, the max-duration
 * auto-stop timer, and the unmount cleanup that must never leak the
 * microphone. All the mic logic that used to live inline in
 * RecordButton, extracted so the component tree only renders.
 */
export function useVoiceCapture(options: {
	isRecording: boolean;
	setIsRecording: (recording: boolean) => void;
	onWavCaptured: (wav: Blob) => void;
	onError: (error: unknown) => void;
	onStopError: (error: unknown) => void;
	onTimeLimit: () => void;
	maxDurationMs?: number;
}) {
	const {
		isRecording,
		setIsRecording,
		onWavCaptured,
		onError,
		onStopError,
		onTimeLimit,
		maxDurationMs = 240_000
	} = options;

	const [status, setStatus] = useState("idle");
	const [micStarting, setMicStarting] = useState(false);
	const [micPermissionDenied, setMicPermissionDenied] = useState(false);

	// Refs so the unmount cleanup always sees the live values.
	const timerRef = useRef<number | null>(null);
	const activeRecordingRef = useRef(false);
	// keep the latest callbacks reachable from the timer/unmount closures
	const callbacksRef = useRef({ onWavCaptured, onError, onStopError, onTimeLimit });
	useEffect(() => {
		callbacksRef.current = { onWavCaptured, onError, onStopError, onTimeLimit };
	});

	// If the component goes away mid-recording (user ends the chat, page
	// navigates), stop the timer and release the Rust-side microphone.
	useEffect(() => {
		return () => {
			if (timerRef.current !== null) {
				clearTimeout(timerRef.current);
			}
			if (activeRecordingRef.current) {
				invoke(COMMANDS.stopVoiceCapture).catch(() => {});
			}
		};
	}, []);

	const stopWavCapture = useCallback(async (): Promise<void> => {
		const wav = await invoke<ArrayBuffer>(COMMANDS.stopVoiceCapture);
		// Only clear the flag once the Rust side has actually stopped:
		// clearing first would make the unmount cleanup skip the stop and
		// leave cpal capturing forever.
		activeRecordingRef.current = false;
		if (!wav || wav.byteLength <= 44) {
			callbacksRef.current.onError("no audio captured");
			return;
		}
		callbacksRef.current.onWavCaptured(new Blob([wav], { type: "audio/wav" }));
	}, []);

	const toggleRecording = useCallback(async (): Promise<void> => {
		if (isRecording) {
			if (timerRef.current !== null) {
				clearTimeout(timerRef.current);
			}
			setStatus("stopping mic...");
			try {
				await stopWavCapture();
				setIsRecording(false);
				setStatus("idle");
			} catch (error) {
				// The stop failed: the mic may still be running, so keep
				// the recording state (the button offers to stop again and
				// the unmount cleanup still fires) and say what happened.
				callbacksRef.current.onStopError(error);
			}
		} else {
			setStatus("starting mic...");
			// flip immediately - the first start in a session can take a
			// moment (CoreAudio device init), and the button should respond
			// instantly; reverted if the mic can't start
			setIsRecording(true);
			setMicStarting(true);
			try {
				await invoke(COMMANDS.startVoiceCapture);
				activeRecordingRef.current = true;
				const timeout = window.setTimeout(() => {
					callbacksRef.current.onTimeLimit();
					stopWavCapture()
						.then(() => setIsRecording(false))
						.catch((error) => callbacksRef.current.onStopError(error));
				}, maxDurationMs);
				timerRef.current = timeout;
				setStatus("recording");
			} catch (error) {
				console.error("Error accessing microphone:", error);
				setIsRecording(false);
				setStatus("idle");
				setMicPermissionDenied(true);
				// surface the raw error through onError (it sets the
				// permission-denied UI's message)
				callbacksRef.current.onError(error);
			} finally {
				setMicStarting(false);
			}
		}
	}, [isRecording, setIsRecording, stopWavCapture, maxDurationMs]);

	const retryAfterPermissionError = useCallback(() => {
		setMicPermissionDenied(false);
		void toggleRecording();
	}, [toggleRecording]);

	return {
		status,
		micStarting,
		micPermissionDenied,
		toggleRecording,
		retryAfterPermissionError,
		secondsRemaining: maxDurationMs / 1000
	};
}
