/** Mic button states and rendering, extracted from RecordButton. */

import { ICON } from "./RecordIcons";
import { CONVERSATION_STATE } from "@src/const";

interface MicButtonProps {
	isRecording: boolean;
	isDisabled: boolean;
	micStarting: boolean;
	conversationState: string;
	elapsedSeconds: number;
	status: string;
	onToggle: () => void;
	onAnimationTrigger: (state: "idle" | "jump") => void;
	isCompressed?: boolean | string | null;
}

export function MicButton({
	isRecording,
	isDisabled,
	micStarting,
	conversationState,
	elapsedSeconds,
	status,
	onToggle,
	onAnimationTrigger,
	isCompressed
}: MicButtonProps) {
	const buttonSizing = isCompressed ? "w-[54px] h-[54px]" : "w-[96px] h-[96px]";
	const shadowSizing = isCompressed
		? "w-[60px] h-[60px] group-hover:w-[66px] group-hover:h-[66px]"
		: "w-[108px] h-[108px] group-hover:w-[116px] group-hover:h-[116px]";

	/** mm:ss readout for the live recording timer */
	const formatElapsed = (totalSeconds: number): string => {
		const minutes = Math.floor(totalSeconds / 60);
		const seconds = totalSeconds % 60;
		return `${minutes}:${String(seconds).padStart(2, "0")}`;
	};

	const disabled =
		micStarting ||
		isDisabled ||
		conversationState === CONVERSATION_STATE.WaitingForCoach ||
		conversationState === CONVERSATION_STATE.TranscribingUser;
	const micAriaLabel = micStarting
		? "Starting microphone"
		: isRecording
			? "Stop recording"
			: "Start recording";

	let icon;
	if (isRecording) {
		icon = ICON.Recording;
	} else {
		icon = ICON.ReadyToRecord;
	}

	return (
		<div>
			<div className="w-[244px] group relative flex justify-center items-center my-4">
				<div
					className={`${
						isRecording && "animate-spin"
					} ${shadowSizing} pointer-events-none group-hover:w-[116px] group-hover:h-[116px] rounded-full absolute bg-gradient-to-r from-pink-500 via-pink-400 to-pink-700 opacity-60 hover:opacity-90 blur transition-all duration-300`}
				></div>
				<button
					disabled={disabled}
					aria-label={micAriaLabel}
					className={`${buttonSizing} focus-visible:ring-4 focus-visible:outline-none focus-visible:ring-pink-300 relative flex items-center justify-center bg-white shadow-xl border-[1px] border-stone-200 text-stone-500 hover:text-stone-700 font-bold rounded-full disabled:opacity-40 disabled:cursor-not-allowed group:hover:scale-105 transform transition-transform hover:bg-stone-100 transition-colors duration-300`}
					onClick={() => {
						onAnimationTrigger(isRecording ? "idle" : "jump");
						onToggle();
					}}
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
					recording &middot; {formatElapsed(elapsedSeconds)}
				</p>
			) : (
				<p>{status === "idle" ? "ready to listen" : status}</p>
			)}
		</div>
	);
}

interface MicPermissionDeniedProps {
	errorMessage: string | null;
	onRetry: () => void;
	isCompressed?: boolean | string | null;
}

export function MicPermissionDenied({
	errorMessage,
	onRetry,
	isCompressed
}: MicPermissionDeniedProps) {
	const buttonSizing = isCompressed ? "w-[54px] h-[54px]" : "w-[96px] h-[96px]";
	const shadowSizing = isCompressed
		? "w-[60px] h-[60px] group-hover:w-[66px] group-hover:h-[66px]"
		: "w-[108px] h-[108px] group-hover:w-[116px] group-hover:h-[116px]";
	return (
		<div>
			<div className="w-[244px] mx-auto group relative flex justify-center items-center my-4">
				<div
					className={`${shadowSizing} pointer-events-none rounded-full absolute bg-gradient-to-r from-pink-500 via-pink-400 to-pink-700 opacity-40 blur transition-all duration-300`}
				></div>
				<button
					className={`${buttonSizing} bg-white relative flex justify-center items-center shadow-xl border-[1px] border-stone-200 text-stone-500 font-bold rounded-full group:hover:scale-105 transform transition-transform hover:bg-stone-100 transition-colors duration-300`}
					onClick={onRetry}
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
}
