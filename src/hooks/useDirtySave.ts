import { useCallback, useRef, useState } from "react";

/**
 * "Saved snapshot + dirty state + save-in-flight" for a settings form.
 * Dirty state is derived from the last CONFIRMED-saved snapshot, so a
 * failed save stays retryable and edits made while a save is pending
 * stay dirty. Only the submitted value becomes clean when the persist
 * resolves true.
 *
 * `equals` decides what counts as unchanged; callers with object or
 * array state pass a structural comparison (Object.is is the default,
 * which only ever matches the very same reference).
 */
export function useDirtySave<T>(
	initial: T,
	equals: (a: T, b: T) => boolean = Object.is
): {
	saved: T;
	isSaving: boolean;
	isDirty: (current: T) => boolean;
	/** Persist `current`; returns whether it was confirmed. */
	save: (current: T, persist: (value: T) => Promise<boolean>) => Promise<boolean>;
} {
	const [saved, setSaved] = useState<T>(initial);
	const [isSaving, setIsSaving] = useState(false);

	const isDirty = useCallback((current: T) => !equals(current, saved), [equals, saved]);

	// The in-flight guard is a ref, not the state: two activations in one
	// tick would both read the stale render's `isSaving`.
	const savingRef = useRef(false);
	const save = useCallback(async (current: T, persist: (value: T) => Promise<boolean>) => {
		if (savingRef.current) return false;
		savingRef.current = true;
		setIsSaving(true);
		try {
			const ok = await persist(current);
			if (ok) setSaved(current);
			return ok;
		} finally {
			savingRef.current = false;
			setIsSaving(false);
		}
	}, []);

	return { saved, isSaving, isDirty, save };
}
