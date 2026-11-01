import { useCallback, useState } from "react";
import { useSnackbar } from "@ds/Snackbar";
import { normalizeApiError } from "@helpers/helpers";

export interface ApiActionOptions {
	/** snackbar copy shown when the action resolves; failures always show
	 * the normalized error (omit for a silent success) */
	success?: string;
	/** pending-flag key: several independent actions can be in flight at
	 * once (one per settings row, per model row) */
	key?: string;
}

/** What `run` resolves with: `ok` distinguishes a resolved action from
 * a failed one (a command's own result can legitimately be null). */
export type ApiOutcome<T> = { ok: true; value: T } | { ok: false };

/**
 * The one wrapper for fire-and-forget API calls: the rejection is
 * normalized and shown through the context snackbar, an optional
 * success message shows on resolve, and a pending flag (optionally
 * keyed) tracks the in-flight work.
 *
 *   const { run, isPending } = useApiAction();
 *   void run(deleteModelApi(id), { success: "Deleted", key: id });
 *
 * `run` never rejects: a failure resolves to `{ ok: false }` once the
 * error has been shown.
 */
export function useApiAction() {
	const { openSnackbar } = useSnackbar();
	const [pending, setPending] = useState<Record<string, boolean>>({});

	const run = useCallback(
		async <T>(
			action: Promise<T>,
			{ success, key }: ApiActionOptions = {}
		): Promise<ApiOutcome<T>> => {
			if (key !== undefined) setPending((prev) => ({ ...prev, [key]: true }));
			try {
				const value = await action;
				if (success !== undefined) openSnackbar(true, success);
				return { ok: true, value };
			} catch (e) {
				openSnackbar(false, normalizeApiError(e));
				return { ok: false };
			} finally {
				if (key !== undefined) setPending((prev) => ({ ...prev, [key]: false }));
			}
		},
		[openSnackbar]
	);

	/** True while the action keyed `key` is in flight. */
	const isPending = useCallback((key: string) => Boolean(pending[key]), [pending]);

	return { run, isPending };
}
