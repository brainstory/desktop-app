/**
 * Data hook for the AI Models card: the catalog, settings, Apple STT
 * availability, free disk space, the download event listener, and the
 * action callbacks. Download *display* state (progress, failures) lives
 * in the shared $downloads store instead of local state, so it survives
 * navigation and is visible outside this page (the sidebar progress
 * pill, the chat setup notice). Engine status comes from the shared
 * $aiStatus store, so a status event re-renders the card without
 * refetching anything; each action reloads only what it changed.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useStore } from "@nanostores/react";
import {
	activateModelApi,
	cancelDownloadApi,
	downloadModelApi,
	getAiSettingsApi,
	getAppleSttStatusApi,
	getFreeDiskSpaceApi,
	listModelsApi,
	pauseDownloadApi,
	saveAiSettingsApi,
	deleteModelApi,
	type AiSettingsResponse,
	type AppleSttStatus,
	type ModelsResponse
} from "@helpers/api/models";
import { getQueryParam, normalizeApiError, safeReturnTarget } from "@helpers/helpers";
import { $aiStatus } from "@components/global/aiStatusStore";
import {
	$downloads,
	clearDownloadFailure,
	formatDownloadError,
	seedDownloads,
	subscribeDownloads
} from "@components/global/downloadStore";
import { EVENTS, QUERY_PARAMS } from "@src/tauri/commands";

/**
 * The only settings fields a secret save can change: the response carries
 * presence + masked hint, never the raw secret. Merging just these keys
 * refreshes the secret controls without clobbering unrelated unsaved
 * endpoint/model edits in the form.
 */
const SECRET_PRESENCE_KEYS = [
	"hfTokenSet",
	"hfTokenHint",
	"extLlmApiKeySet",
	"extLlmApiKeyHint",
	"extSttApiKeySet",
	"extSttApiKeyHint"
] as const;

/** Merge only the secret presence/hint fields of `fresh` into `base`. */
const mergeSecretPresence = (
	base: AiSettingsResponse,
	fresh: AiSettingsResponse
): AiSettingsResponse => ({
	...base,
	...(Object.fromEntries(
		SECRET_PRESENCE_KEYS.map((key) => [key, fresh[key]])
	) as Partial<AiSettingsResponse>)
});

export function useAiModels(openSnackbar: (isSuccess: boolean, message: string) => void) {
	// the first-launch detour: where "back to your idea" goes once the
	// downloaded model is ready (null when no valid target was given)
	const backTarget = useMemo(() => safeReturnTarget(getQueryParam(QUERY_PARAMS.return)), []);
	const [downloadLanded, setDownloadLanded] = useState(false);
	const [backDismissed, setBackDismissed] = useState(false);
	const [models, setModels] = useState<ModelsResponse>({ llm: [], stt: [] });
	const [settings, setSettings] = useState<AiSettingsResponse | null>(null);
	/** the settings read failed: the card shows the error + a retry
	 * instead of spinning on "Loading..." forever */
	const [settingsError, setSettingsError] = useState<string | null>(null);
	/** the model catalog read failed (the list would render empty) */
	const [modelsError, setModelsError] = useState(false);
	/** last persisted snapshot of the endpoint fields, for dirty tracking */
	const [savedSettings, setSavedSettings] = useState<AiSettingsResponse | null>(null);
	const runtime = useStore($aiStatus);
	const {
		active: activeDownloads,
		paused: pausedDownloadEntries,
		failures: downloadFailures
	} = useStore($downloads);
	const [appleStt, setAppleStt] = useState<AppleSttStatus | null>(null);
	const [freeBytes, setFreeBytes] = useState<number | null>(null);
	/** long-running per-row action, so the buttons can say "Deleting…"
	 * instead of appearing dead while a load is waited out */
	const [rowPending, setRowPending] = useState<Record<string, "delete" | "activate">>({});

	/** Catalog + downloaded/active flags (downloads, deletes, activation, mode). */
	const refreshModels = useCallback(() => {
		listModelsApi()
			.then((data) => {
				setModels(data);
				setModelsError(false);
				// Rehydrate in-flight and paused downloads (e.g. after
				// navigating away and back). The listing is a snapshot; the
				// live event stream is fresher, so it only fills ids the
				// store does not track yet.
				const entries: Record<string, { pct: number; label: string }> = {};
				const pausedEntries: Record<string, { pct: number; label: string }> = {};
				for (const list of [data.llm, data.stt]) {
					for (const model of list) {
						if (model.downloading) {
							entries[model.id] = {
								pct: model.progress ?? -1,
								label: model.label
							};
						} else if (model.paused) {
							pausedEntries[model.id] = {
								pct: model.progress ?? -1,
								label: model.label
							};
						}
					}
				}
				seedDownloads(entries, pausedEntries);
			})
			.catch((e) => {
				console.error("list models failed", e);
				setModelsError(true);
			});
	}, []);

	/** Free space on the models volume (downloads and deletions change it). */
	const refreshDiskSpace = useCallback(() => {
		getFreeDiskSpaceApi()
			.then(setFreeBytes)
			.catch((e) => console.error("free disk space failed", e));
	}, []);

	/** Read the settings into the form. Also the retry path, so a failed
	 * initial read is recoverable without reloading the page. */
	const loadSettings = useCallback(() => {
		getAiSettingsApi()
			.then((data) => {
				setSettings(data);
				setSavedSettings(data);
				setSettingsError(null);
			})
			.catch((e) => {
				console.error("ai settings failed", e);
				setSettingsError(normalizeApiError(e));
			});
	}, []);

	const retrySettings = useCallback(() => {
		setSettingsError(null);
		loadSettings();
	}, [loadSettings]);

	/** Re-sync only the saved snapshot (the dirty flag's reference);
	 * `settings` keeps the live form state. */
	const reloadSavedSettings = () => {
		getAiSettingsApi()
			.then((saved) => setSavedSettings(saved))
			.catch((e) => console.error("failed to reload ai settings", e));
	};

	useEffect(() => {
		loadSettings();
		refreshModels();
		refreshDiskSpace();
		// a system capability: it doesn't change while the page is open
		getAppleSttStatusApi()
			.then(setAppleStt)
			.catch((e) => console.error("apple stt status failed", e));

		// The shared feed owns display state (progress, failures,
		// cancels); this page adds its side effects on top. The feed is
		// refcounted and lives exactly as long as something needs it.
		const unsubscribe = subscribeDownloads();
		const unlisten = listen<{
			modelId: string;
			label?: string;
			kind:
				"progress" | "paused" | "done" | "cancelled" | "error" | "load-error" | "activated";
			pct?: number;
			message?: string;
		}>(EVENTS.modelDownload, (event) => {
			const { kind, message } = event.payload ?? {};
			if (kind === "done") {
				setDownloadLanded(true);
				openSnackbar(true, "Model downloaded");
				refreshModels();
				refreshDiskSpace();
			} else if (kind === "paused") {
				// the paused flag and stop point live in the listing; the
				// row's Resume button reads them from there
				refreshModels();
			} else if (kind === "activated") {
				// the selection moved to the finished download: the
				// "Active" badge must follow
				refreshModels();
			} else if (kind === "error") {
				openSnackbar(false, formatDownloadError(message ?? ""));
			} else if (kind === "load-error") {
				// the download itself succeeded; activating the model failed
				openSnackbar(false, `Model downloaded, but turning it on failed: ${message}`);
				refreshModels();
				refreshDiskSpace();
			}
			// "progress" and "cancelled" change no catalog data: nothing to
			// refresh, and a cancel is deliberately not toasted.
		});
		return () => {
			unsubscribe();
			void unlisten.then((fn) => fn());
		};
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, []);

	// the per-row actions keep stable identities for the memoised ModelRow
	const download = useCallback(
		(modelId: string): void => {
			// a retry supersedes the recorded failure
			clearDownloadFailure(modelId);
			downloadModelApi(modelId).invokePromise.catch((e) => {
				// starting a download that's already running is harmless - the
				// progress bar is already driven by backend events
				if (!String(e).includes("already downloading")) {
					openSnackbar(false, normalizeApiError(e));
				}
				refreshModels();
			});
		},
		[openSnackbar, refreshModels]
	);

	/**
	 * Immediate single-action save (toggles, engine chips, language).
	 * Sends ONLY the changed keys: the backend validates every key it
	 * receives, so spreading the whole form persisted half-typed endpoint
	 * fields and let one stale stored value fail every save. A failure
	 * reverts just these keys to their previous values.
	 */
	const save = (updates: Partial<AiSettingsResponse>): void => {
		if (!settings) return;
		const keys = Object.keys(updates) as (keyof AiSettingsResponse)[];
		const previous = Object.fromEntries(
			keys.map((key) => [key, settings[key]])
		) as Partial<AiSettingsResponse>;
		setSettings((prev) => (prev ? { ...prev, ...updates } : prev));
		saveAiSettingsApi(updates)
			.then(() => {
				setSavedSettings((prev) => (prev ? { ...prev, ...updates } : prev));
				// mode/engine changes move the active flags
				refreshModels();
			})
			.catch((e) => {
				setSettings((prev) => (prev ? { ...prev, ...previous } : prev));
				openSnackbar(false, normalizeApiError(e));
			});
	};

	// Secrets: value "" is the backend's clear signal; anything else sets.
	// The stored value is never round-tripped through the UI: it lives only
	// in the transient input/submission, and the response carries presence
	// + masked hint. Resolves true only once the backend confirmed the
	// save, so the control can clear the typed input on success and keep
	// it for a retry on failure.
	const saveSecret = (key: string, value: string): Promise<boolean> => {
		if (!settings) return Promise.resolve(false);
		// Send ONLY the secret: the Rust command applies present keys, and
		// the spread form would persist whatever is sitting half-typed in
		// the endpoint inputs right now.
		return (
			saveAiSettingsApi({ [key]: value })
				.then(() => {
					openSnackbar(true, value === "" ? "Removed" : "Saved");
					// Confirmed. Re-read the settings and merge ONLY the
					// presence/hint fields into both the saved snapshot and the
					// displayed form, so the controls show the stored state
					// without discarding unsaved endpoint/model edits.
					return getAiSettingsApi()
						.then((fresh) => {
							setSavedSettings((prev) =>
								prev ? mergeSecretPresence(prev, fresh) : fresh
							);
							setSettings((prev) => (prev ? mergeSecretPresence(prev, fresh) : prev));
						})
						.catch((e) => {
							// The secret IS stored; only the confirmation read
							// failed. Don't fake the presence it implies - just
							// surface the refresh failure.
							console.error("failed to reload ai settings", e);
							openSnackbar(
								false,
								`${value === "" ? "Removed" : "Saved"}, but re-reading the settings failed: ${normalizeApiError(e)}`
							);
						});
				})
				// the refresh outcome above never rejects; true = confirmed save
				.then(() => true)
				.catch((e) => {
					openSnackbar(false, normalizeApiError(e));
					return false;
				})
		);
	};

	/** Mirror endpoint row sends ONLY its field (like the secret rows). */
	const saveEndpoint = (value: string): void => {
		if (!settings) return;
		saveAiSettingsApi({ hfEndpoint: value })
			.then(() => {
				reloadSavedSettings();
				openSnackbar(true, value.trim() ? "Mirror saved" : "Mirror cleared");
			})
			.catch((e) => openSnackbar(false, normalizeApiError(e)));
	};

	const deleteModel = useCallback(
		(modelId: string): void => {
			setRowPending((prev) => ({ ...prev, [modelId]: "delete" }));
			deleteModelApi(modelId)
				.then(() => {
					refreshModels();
					refreshDiskSpace();
				})
				.catch((e) => openSnackbar(false, normalizeApiError(e)))
				.finally(() =>
					setRowPending((prev) => {
						const next = { ...prev };
						delete next[modelId];
						return next;
					})
				);
		},
		[openSnackbar, refreshModels, refreshDiskSpace]
	);

	const cancelDownload = useCallback(
		(modelId: string): void => {
			cancelDownloadApi(modelId).catch((e) => openSnackbar(false, normalizeApiError(e)));
		},
		[openSnackbar]
	);

	const pauseDownload = useCallback(
		(modelId: string): void => {
			pauseDownloadApi(modelId).catch((e) => openSnackbar(false, normalizeApiError(e)));
		},
		[openSnackbar]
	);

	const activateModel = useCallback(
		(modelId: string): void => {
			setRowPending((prev) => ({ ...prev, [modelId]: "activate" }));
			activateModelApi(modelId)
				.then(refreshModels)
				.catch((e) => openSnackbar(false, normalizeApiError(e)))
				.finally(() =>
					setRowPending((prev) => {
						const next = { ...prev };
						delete next[modelId];
						return next;
					})
				);
		},
		[openSnackbar, refreshModels]
	);

	// Row-shaped projection of the store (ModelRow reads plain numbers)
	const downloadProgress: Record<string, number> = {};
	const downloadRates: Record<string, { bytesPerSec: number; etaSecs: number | null }> = {};
	for (const [modelId, entry] of Object.entries(activeDownloads)) {
		downloadProgress[modelId] = entry.pct;
		if (entry.bytesPerSec !== undefined) {
			downloadRates[modelId] = {
				bytesPerSec: entry.bytesPerSec,
				etaSecs: entry.etaSecs ?? null
			};
		}
	}
	const pausedDownloads: Record<string, number> = {};
	for (const [modelId, entry] of Object.entries(pausedDownloadEntries)) {
		pausedDownloads[modelId] = entry.pct;
	}

	return {
		// the ready banner: only after a download finished here AND the
		// runtime reports the LLM ready (activation can still fail)
		backHref:
			downloadLanded && !backDismissed && runtime.llm.state === "ready" ? backTarget : null,
		dismissBack: () => setBackDismissed(true),
		// state
		models,
		settings,
		setSettings,
		savedSettings,
		setSavedSettings,
		runtime,
		downloadProgress,
		downloadRates,
		pausedDownloads,
		downloadFailures,
		rowPending,
		appleStt,
		freeBytes,
		settingsError,
		modelsError,
		// actions
		refreshModels,
		retrySettings,
		download,
		save,
		saveSecret,
		saveEndpoint,
		deleteModel,
		cancelDownload,
		pauseDownload,
		activateModel
	};
}
