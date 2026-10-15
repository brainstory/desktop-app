import {
	createContext,
	useCallback,
	useContext,
	useEffect,
	useRef,
	useState,
	type ReactNode
} from "react";

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
	/** optional inline action, e.g. an "Open" link to what just happened */
	action?: { label: string; href: string };
}

export function Snackbar({ isSuccess, message, onClose, action }: SnackbarProps) {
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
		? "bg-green-700 z-50 fixed top-4 left-1/2 -translate-x-1/2 mt-0 p-4 pr-12 rounded-md shadow-lg text-center max-w-[90vw]"
		: "bg-red-600 z-50 fixed top-4 left-1/2 -translate-x-1/2 mt-0 p-4 pr-12 rounded-md shadow-lg text-center max-w-[90vw]";

	return (
		<div
			role={isSuccess ? "status" : "alert"}
			aria-live={isSuccess ? "polite" : "assertive"}
			className={style}
		>
			<p className="text-white inline">
				{message}{" "}
				{action && (
					<a href={action.href} className="underline font-semibold whitespace-nowrap">
						{action.label}
					</a>
				)}
			</p>
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

/** What the island-wide snackbar host exposes: `openSnackbar` shows a
 * message; the single host (mounted by SnackbarProvider) renders it.
 * A success and an error can be open at once, each dismissing itself.
 *
 *   const { openSnackbar } = useSnackbar();
 *   openSnackbar(false, "Could not save");
 */
interface SnackbarContextValue {
	openSnackbar: (
		isSuccess: boolean,
		message: string,
		action?: { label: string; href: string }
	) => void;
}

const SnackbarContext = createContext<SnackbarContextValue>({
	openSnackbar: () => {}
});

/**
 * One snackbar host per island root. Without it, every card rendered
 * its own fixed-position snackbar and two of them stacked on top of
 * each other; with it, one queue per page.
 */
export function SnackbarProvider({ children }: { children: ReactNode }) {
	const [successMessage, setSuccessMessage] = useState<string | null>(null);
	const [successAction, setSuccessAction] = useState<{ label: string; href: string } | undefined>(
		undefined
	);
	const [errorMessage, setErrorMessage] = useState<string | null>(null);

	const openSnackbar = useCallback(
		(isSuccess: boolean, message: string, action?: { label: string; href: string }): void => {
			if (isSuccess) {
				setSuccessMessage(message);
				setSuccessAction(action);
			} else {
				setErrorMessage(message);
			}
		},
		[]
	);

	return (
		<SnackbarContext.Provider value={{ openSnackbar }}>
			{children}
			{successMessage !== null && (
				<Snackbar
					isSuccess={true}
					message={successMessage}
					action={successAction}
					onClose={() => {
						setSuccessMessage(null);
						setSuccessAction(undefined);
					}}
				/>
			)}
			{errorMessage !== null && (
				<Snackbar
					isSuccess={false}
					message={errorMessage}
					onClose={() => setErrorMessage(null)}
				/>
			)}
		</SnackbarContext.Provider>
	);
}

export function useSnackbar(): SnackbarContextValue {
	return useContext(SnackbarContext);
}

export default {
	Snackbar,
	SUCCESS_COPY,
	ERROR_COPY
};
