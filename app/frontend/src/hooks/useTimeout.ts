import { useCallback, useEffect, useRef, useState } from "react";

/**
 * A timeout that is always cleared on unmount and whenever it is
 * re-scheduled. Returns [schedule, cancel] as stable identities.
 */
export function useTimeout(): readonly [
	schedule: (fn: () => void, ms: number) => void,
	cancel: () => void
] {
	const timerRef = useRef<number | null>(null);

	const cancel = useCallback(() => {
		if (timerRef.current !== null) {
			window.clearTimeout(timerRef.current);
			timerRef.current = null;
		}
	}, []);

	const schedule = useCallback(
		(fn: () => void, ms: number) => {
			cancel();
			timerRef.current = window.setTimeout(() => {
				timerRef.current = null;
				fn();
			}, ms);
		},
		[cancel]
	);

	// never leave a timer running past this component's life
	useEffect(() => cancel, [cancel]);

	return [schedule, cancel] as const;
}

export const CONFIRM_RESET_MS = 5000;

/** Screen-reader text for an armed delete: put it in a live region
 * OUTSIDE the button, once per arming (not per countdown tick). */
export const CONFIRM_DELETE_ANNOUNCEMENT = `Click again to delete, resets in ${CONFIRM_RESET_MS / 1000} seconds`;

/**
 * Two-click confirmation for destructive actions: the first click arms
 * (and auto-resets after `resetMs`), the second click within the window
 * confirms. The reset timer never survives unmount.
 */
export function useConfirmClick(resetMs = CONFIRM_RESET_MS): {
	isConfirming: boolean;
	/** Seconds until the confirmation auto-resets (for a visible countdown). */
	secondsLeft: number | null;
	/** Returns true on the confirming (second) click. */
	confirm: () => boolean;
	reset: () => void;
} {
	const [isConfirming, setIsConfirming] = useState(false);
	const [secondsLeft, setSecondsLeft] = useState<number | null>(null);
	const [schedule, cancel] = useTimeout();

	const confirm = useCallback((): boolean => {
		if (!isConfirming) {
			setIsConfirming(true);
			setSecondsLeft(Math.round(resetMs / 1000));
			schedule(() => {
				setIsConfirming(false);
				setSecondsLeft(null);
			}, resetMs);
			return false;
		}
		cancel();
		setIsConfirming(false);
		setSecondsLeft(null);
		return true;
	}, [isConfirming, schedule, cancel, resetMs]);

	// tick the visible countdown once a second while armed
	useEffect(() => {
		if (!isConfirming) return;
		const tick = window.setInterval(() => {
			setSecondsLeft((s) => (s === null ? s : Math.max(0, s - 1)));
		}, 1000);
		return () => window.clearInterval(tick);
	}, [isConfirming]);

	const reset = useCallback(() => {
		cancel();
		setIsConfirming(false);
		setSecondsLeft(null);
	}, [cancel]);

	return { isConfirming, secondsLeft, confirm, reset };
}
