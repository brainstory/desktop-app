import { useCallback, useEffect, useRef, useState } from "react";
import { invokeCommand } from "@src/tauri/invoke";

/**
 * Rust-side voice capture lifecycle: start/stop cpal, the max-duration
 * auto-stop timer, and the unmount cleanup that must never leak the
 * microphone. All the mic logic that used to live inline in
 * RecordButton, extracted so the component tree only renders.
 */

// Exact Rust-side error strings from src-tauri/src/voice.rs
// stop_capture. "not recording" = idempotent already-stopped; "no
// audio captured" = the device was ALREADY released (stop_capture
// drops the stream before WAV encoding). Both mean there is nothing
// left to stop, so the UI returns to idle instead of a retry that can
// never succeed; real stop failures keep the retry UI. A typed IPC
// error enum was considered and deferred - these strings are the
// contract and are pinned by Rust tests in voice.rs.
const NOT_RECORDING = "not recording";
const NO_AUDIO_CAPTURED = "no audio captured";

function wasAlreadyReleased(error: unknown): boolean {
	const message =
		typeof error === "string"
			? error
			: ((error as { message?: unknown } | null)?.message ?? error);
	return message === NOT_RECORDING || message === NO_AUDIO_CAPTURED;
}

export function useVoiceCapture(options: {
	isRecording: boolean;
	setIsRecording: (recording: boolean) => void;
	onWavCaptured: (wav: Blob) => void;
	onError: (error: unknown) => void;
	onStopError: (error: unknown) => void;
	onTimeLimit: () => void;
	/** the capture actually started / stopped (for announcements) */
	onRecordingChange?: (change: "started" | "stopped") => void;
	maxDurationMs?: number;
}) {
	const {
		isRecording,
		setIsRecording,
		onWavCaptured,
		onError,
		onStopError,
		onTimeLimit,
		onRecordingChange,
		maxDurationMs = 240_000
	} = options;

	const [status, setStatus] = useState("idle");
	const [micStarting, setMicStarting] = useState(false);
	const [micPermissionDenied, setMicPermissionDenied] = useState(false);

	// Refs so the unmount cleanup always sees the live values.
	const timerRef = useRef<number | null>(null);
	const activeRecordingRef = useRef(false);
	const mountedRef = useRef(true);
	// set by page teardown, so a start that resolves afterwards
	// releases the mic instead of arming the timer on a page that is
	// already going away
	const tornDownRef = useRef(false);
	// keep the latest callbacks reachable from the timer/unmount closures
	const callbacksRef = useRef({
		onWavCaptured,
		onError,
		onStopError,
		onTimeLimit,
		onRecordingChange
	});
	useEffect(() => {
		callbacksRef.current = {
			onWavCaptured,
			onError,
			onStopError,
			onTimeLimit,
			onRecordingChange
		};
	});

	// Shared teardown for pagehide and React unmount. Idempotent: the
	// active flag flips before the stop is fired, so pagehide + unmount
	// (either order, or twice) never double-stop. The stop's WAV bytes
	// are discarded - never onWavCaptured from teardown, a dying page
	// must not kick off a transcription. Native abandonment in voice.rs
	// (wired to page load / window destroy / app exit) is the backstop
	// for when this JS never runs at all.
	const teardownRecording = useCallback(() => {
		tornDownRef.current = true;
		if (timerRef.current !== null) {
			clearTimeout(timerRef.current);
			timerRef.current = null;
		}
		if (activeRecordingRef.current) {
			activeRecordingRef.current = false;
			invokeCommand("stopVoiceCapture").catch(() => {});
		}
	}, []);

	// If the component goes away mid-recording (user ends the chat),
	// stop the timer and release the Rust-side microphone.
	useEffect(() => {
		mountedRef.current = true;
		return () => {
			mountedRef.current = false;
			teardownRecording();
		};
	}, [teardownRecording]);

	// Astro full-document navigation can discard the page without ever
	// running React unmount cleanup; pagehide is the browser's own
	// "this page is going away" signal, so it gets the same best-effort
	// teardown (see teardownRecording for the native backstop).
	useEffect(() => {
		const onPageHide = () => teardownRecording();
		window.addEventListener("pagehide", onPageHide);
		return () => window.removeEventListener("pagehide", onPageHide);
	}, [teardownRecording]);

	const stopWavCapture = useCallback(async (): Promise<void> => {
		const wav = await invokeCommand("stopVoiceCapture");
		// Only clear the flag once the Rust side has actually stopped:
		// clearing first would make the unmount cleanup skip the stop and
		// leave cpal capturing forever.
		activeRecordingRef.current = false;
		// before the WAV handoff, which may announce the transcription
		callbacksRef.current.onRecordingChange?.("stopped");
		if (!wav || wav.byteLength <= 44) {
			callbacksRef.current.onError(NO_AUDIO_CAPTURED);
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
				if (wasAlreadyReleased(error)) {
					// The device is already released - "not recording"
					// (already stopped, idempotent) or "no audio
					// captured" (stream dropped before WAV encoding).
					// Return to idle and report it: staying in "try
					// stopping again" can never succeed against an
					// already-stopped device.
					activeRecordingRef.current = false;
					setIsRecording(false);
					setStatus("idle");
					callbacksRef.current.onError(error);
					return;
				}
				// The stop failed: the mic may still be running, so keep
				// the recording state (the button offers to stop again and
				// the unmount cleanup still fires) and say what happened.
				setStatus("couldn't stop the mic - try again");
				callbacksRef.current.onStopError(error);
			}
		} else {
			setStatus("starting mic...");
			// flip immediately - the first start in a session can take a
			// moment (CoreAudio device init), and the button should respond
			// instantly; reverted if the mic can't start
			setIsRecording(true);
			setMicStarting(true);
			// a fresh recording clears any earlier teardown state
			tornDownRef.current = false;
			try {
				await invokeCommand("startVoiceCapture");
				if (!mountedRef.current || tornDownRef.current) {
					// unmounted or the page began going away while the
					// start was in flight: cleanup already ran and saw
					// no active recording, so release the mic now
					// instead of arming the 4-minute timer
					invokeCommand("stopVoiceCapture").catch(() => {});
					return;
				}
				activeRecordingRef.current = true;
				const timeout = window.setTimeout(() => {
					callbacksRef.current.onTimeLimit();
					stopWavCapture()
						.then(() => setIsRecording(false))
						.catch((error) => {
							if (wasAlreadyReleased(error)) {
								// already released (see toggleRecording):
								// back to idle and reported, not stuck
								// in "recording"
								setIsRecording(false);
								setStatus("idle");
								callbacksRef.current.onError(error);
							} else {
								callbacksRef.current.onStopError(error);
							}
						});
				}, maxDurationMs);
				timerRef.current = timeout;
				setStatus("recording");
				callbacksRef.current.onRecordingChange?.("started");
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
