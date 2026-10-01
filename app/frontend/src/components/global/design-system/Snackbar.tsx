import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

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
	// Parents pass inline onClose arrows (new identity every render);
	// read it through a ref so the auto-dismiss timer is keyed on the
	// message only - a parent re-render must not restart the countdown.
	const onCloseRef = useRef(onClose);
	useEffect(() => {
		onCloseRef.current = onClose;
	}, [onClose]);

	useEffect(() => {
		const timer = setTimeout(() => onCloseRef.current(), AUTO_DISMISS_MS);
		return () => clearTimeout(timer);
	}, [message]);

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

/**
 * Success + error snackbar state for one screen. Render `snackbars`
 * where the Snackbar elements belong; `openSnackbar` (stable identity)
 * shows a message. A success and an error can be open at once, each
 * dismissing itself.
 *
 *   const { openSnackbar, snackbars } = useSnackbar();
 *   openSnackbar(false, "Could not save");
 */
export function useSnackbar(): {
	openSnackbar: (isSuccess: boolean, message: string) => void;
	snackbars: ReactNode;
} {
	const [successMessage, setSuccessMessage] = useState<string | null>(null);
	const [errorMessage, setErrorMessage] = useState<string | null>(null);

	const openSnackbar = useCallback((isSuccess: boolean, message: string): void => {
		if (isSuccess) {
			setSuccessMessage(message);
		} else {
			setErrorMessage(message);
		}
	}, []);

	const snackbars = (
		<>
			{successMessage !== null && (
				<Snackbar
					isSuccess={true}
					message={successMessage}
					onClose={() => setSuccessMessage(null)}
				/>
			)}
			{errorMessage !== null && (
				<Snackbar
					isSuccess={false}
					message={errorMessage}
					onClose={() => setErrorMessage(null)}
				/>
			)}
		</>
	);

	return { openSnackbar, snackbars };
}

export default {
	Snackbar,
	SUCCESS_COPY,
	ERROR_COPY
};
