import { useCallback, useSyncExternalStore } from "react";

/**
 * Live `window.matchMedia(query).matches` with a single change listener.
 *
 * `serverValue` is what the server render (and the first hydration pass)
 * sees, so SSR'd islands never mismatch; the real value takes over right
 * after hydration. Pick the value that is safe before JS runs.
 */
export function useMediaQuery(query: string, serverValue = false): boolean {
	const subscribe = useCallback(
		(onChange: () => void) => {
			if (typeof window === "undefined" || !window.matchMedia) return () => {};
			const mql = window.matchMedia(query);
			mql.addEventListener("change", onChange);
			return () => mql.removeEventListener("change", onChange);
		},
		[query]
	);

	const getSnapshot = (): boolean =>
		typeof window !== "undefined" && !!window.matchMedia
			? window.matchMedia(query).matches
			: serverValue;

	return useSyncExternalStore(subscribe, getSnapshot, () => serverValue);
}
