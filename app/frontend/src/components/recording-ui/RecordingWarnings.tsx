/** Text input mode + warning banners, extracted from RecordButton. */

import { useState } from "react";

export function TextComposer({
	value,
	onChange,
	onSend
}: {
	value: string;
	onChange: (next: string) => void;
	onSend: () => void;
}) {
	const handleKeyDown = (event: React.KeyboardEvent): void => {
		// Enter sends; Shift+Enter inserts a newline (the field is a
		// multi-line textarea, so users expect both)
		if (event.key === "Enter" && !event.shiftKey) {
			event.preventDefault();
			onSend();
		}
	};
	return (
		<div className="flex justify-center items-end w-full">
			<div className="flex flex-col w-full max-w-[500px]">
				<textarea
					className="w-full text-sm px-4 py-2 border border-stone-200 rounded-md focus:outline-none focus:border-accent-500 resize-none md:resize-y"
					placeholder="Type something..."
					aria-label="Type your response"
					rows={5}
					value={value}
					onChange={(e) => onChange(e.target.value)}
					onKeyDown={handleKeyDown}
				></textarea>
				<p className="w-full max-w-[500px] text-xs text-stone-500 text-left mt-1">
					Enter to send &middot; Shift+Enter for a new line
				</p>
			</div>
			<button
				className="flex items-center h-[36px] w-[36px] ml-2 p-2 rounded-full bg-accent-600 text-white hover:bg-accent-700 focus:outline-none focus:ring focus:border-accent-400 disabled:opacity-40 disabled:cursor-not-allowed"
				aria-label="Send message"
				disabled={!value.trim()}
				onClick={onSend}
			>
				<ion-icon class="w-8 h-8 hydrated" name="send" aria-hidden="true"></ion-icon>
			</button>
		</div>
	);
}

const warningBoxStyle =
	"flex justify-center fixed z-50 left-0 right-0 w-3/4 mx-auto bg-red-700 text-white py-4 px-4 rounded-lg shadow-lg flex items-center top-8";

export function RecordingWarnings({
	warningType,
	errorMessage,
	onDismiss,
	secondsRemaining,
	isRecording
}: {
	warningType: string | null;
	errorMessage: string | null;
	onDismiss: () => void;
	secondsRemaining: number;
	isRecording: boolean;
}) {
	return (
		<>
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
						onClick={onDismiss}
					>
						×
					</button>
				</div>
			)}
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

/** local state bundle for the warning banner */
export function useRecordingWarnings() {
	const [warningType, setWarningType] = useState<string | null>(null);
	const [errorMessage, setErrorMessage] = useState<string | null>(null);
	const dismiss = () => {
		setWarningType(null);
		setErrorMessage(null);
	};
	return { warningType, setWarningType, errorMessage, setErrorMessage, dismiss };
}
