import { useCallback, useEffect, useRef } from "react";

/**
 * An AbortSignal that aborts when the component unmounts, for cancelling
 * timers and retries that would otherwise outlive it.
 *
 * Returns a getter rather than the signal: call it from event handlers
 * and effects (not during render) so a StrictMode remount, which gets a
 * fresh controller, is picked up. After unmount it returns the aborted
 * signal.
 */
export function useUnmountSignal(): () => AbortSignal {
	const controllerRef = useRef<AbortController | null>(null);

	useEffect(() => {
		// a child effect may already have asked for the signal: keep it
		if (!controllerRef.current || controllerRef.current.signal.aborted) {
			controllerRef.current = new AbortController();
		}
		const controller = controllerRef.current;
		return () => controller.abort();
	}, []);

	return useCallback(() => {
		controllerRef.current ??= new AbortController();
		return controllerRef.current.signal;
	}, []);
}
