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

/**
 * Two-click confirmation for destructive actions: the first click arms
 * (and auto-resets after `resetMs`), the second click within the window
 * confirms. The reset timer never survives unmount.
 */
export function useConfirmClick(resetMs = 5000): {
	isConfirming: boolean;
	/** Returns true on the confirming (second) click. */
	confirm: () => boolean;
	reset: () => void;
} {
	const [isConfirming, setIsConfirming] = useState(false);
	const [schedule, cancel] = useTimeout();

	const confirm = useCallback((): boolean => {
		if (!isConfirming) {
			setIsConfirming(true);
			schedule(() => setIsConfirming(false), resetMs);
			return false;
		}
		cancel();
		setIsConfirming(false);
		return true;
	}, [isConfirming, schedule, cancel, resetMs]);

	const reset = useCallback(() => {
		cancel();
		setIsConfirming(false);
	}, [cancel]);

	return { isConfirming, confirm, reset };
}
