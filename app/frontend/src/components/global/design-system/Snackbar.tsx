import { useEffect } from "react";

export const SUCCESS_COPY = {
	DEFAULT: "Success!",
	SAVE: "Changes Saved!"
};

export const ERROR_COPY = {
	DEFAULT: "Something went wrong. Please try again.",
	SAVE: "Error saving changes. Please try again."
};

const AUTO_DISMISS_MS = 5000;

interface SnackbarProps {
	isSuccess: boolean;
	message: string;
	/** Called when the snackbar is dismissed (button or auto-dismiss). */
	onClose: () => void;
}

export function Snackbar({ isSuccess, message, onClose }: SnackbarProps) {
	// Auto-dismiss in an effect (never during render), and a dismissal
	// timer always belongs to exactly one mounted snackbar.
	useEffect(() => {
		const timer = setTimeout(onClose, AUTO_DISMISS_MS);
		return () => clearTimeout(timer);
	}, [onClose]);

	const style = isSuccess
		? "bg-green-600 z-50 fixed top-4 left-1/2 -translate-x-1/2 mt-0 p-4 pr-12 rounded-md shadow-lg text-center max-w-[90vw]"
		: "bg-red-600 z-50 fixed top-4 left-1/2 -translate-x-1/2 mt-0 p-4 pr-12 rounded-md shadow-lg text-center max-w-[90vw]";

	return (
		<div role="status" aria-live="polite" className={style}>
			<p className="text-white">{message}</p>
			<button
				className="absolute top-1/2 right-2 -translate-y-1/2 text-white/70 hover:text-white text-lg font-bold px-2"
				aria-label="Dismiss message"
				onClick={onClose}
			>
				×
			</button>
		</div>
	);
}

export default {
	Snackbar,
	SUCCESS_COPY,
	ERROR_COPY
};
