/**
 * Data hook for the AI Models card: all state (catalog, settings,
 * download progress, Apple STT availability, free disk space), the
 * download event listener, and the action callbacks. Engine status comes
 * from the shared $aiStatus store, so a status event re-renders the card
 * without refetching anything; each action reloads only what it changed.
 */

import { useCallback, useEffect, useState } from "react";
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
	saveAiSettingsApi,
	deleteModelApi,
	type AiSettingsResponse,
	type AppleSttStatus,
	type ModelsResponse
} from "@helpers/api/models";
import { normalizeApiError } from "@helpers/helpers";
import { $aiStatus } from "@components/global/aiStatusStore";

export function useAiModels(openSnackbar: (isSuccess: boolean, message: string) => void) {
	const [models, setModels] = useState<ModelsResponse>({ llm: [], stt: [] });
	const [settings, setSettings] = useState<AiSettingsResponse | null>(null);
	/** last persisted snapshot of the endpoint fields, for dirty tracking */
	const [savedSettings, setSavedSettings] = useState<AiSettingsResponse | null>(null);
	const runtime = useStore($aiStatus);
	const [downloadProgress, setDownloadProgress] = useState<Record<string, number>>({});
	const [appleStt, setAppleStt] = useState<AppleSttStatus | null>(null);
	const [freeBytes, setFreeBytes] = useState<number | null>(null);

	/** Catalog + downloaded/active flags (downloads, deletes, activation, mode). */
	const refreshModels = useCallback(() => {
		listModelsApi()
			.then((data) => {
				setModels(data);
				// rehydrate in-flight downloads (e.g. after navigating away and back)
				const next: Record<string, number> = {};
				for (const list of [data.llm, data.stt]) {
					for (const model of list) {
						if (model.downloading) {
							next[model.id] = model.progress ?? 0;
						}
					}
				}
				setDownloadProgress(next);
			})
			.catch((e) => console.error("list models failed", e));
	}, []);

	/** Free space on the models volume (downloads and deletions change it). */
	const refreshDiskSpace = useCallback(() => {
		getFreeDiskSpaceApi()
			.then(setFreeBytes)
			.catch((e) => console.error("free disk space failed", e));
	}, []);

	/** Re-sync only the saved snapshot (the dirty flag's reference);
	 * `settings` keeps the live form state. */
	const reloadSavedSettings = () => {
		getAiSettingsApi()
			.then((saved) => setSavedSettings(saved))
			.catch((e) => console.error("failed to reload ai settings", e));
	};

	useEffect(() => {
		getAiSettingsApi()
			.then((data) => {
				setSettings(data);
				setSavedSettings(data);
			})
			.catch((e) => console.error("ai settings failed", e));
		refreshModels();
		refreshDiskSpace();
		// a system capability: it doesn't change while the page is open
		getAppleSttStatusApi()
			.then(setAppleStt)
			.catch((e) => console.error("apple stt status failed", e));

		// downloads continue in the backend across page navigation
		const unlisten = listen<{
			modelId: string;
			kind: "progress" | "done" | "error" | "load-error";
			pct?: number;
			message?: string;
		}>("model-download", (event) => {
			const { modelId, kind, pct, message } = event.payload ?? {};
			if (kind === "progress") {
				setDownloadProgress((prev: Record<string, number>) => ({
					...prev,
					[modelId]: pct ?? 0
				}));
			} else if (kind === "done") {
				setDownloadProgress((prev) => {
					const next = { ...prev };
					delete next[modelId];
					return next;
				});
				openSnackbar(true, "Model downloaded");
				refreshModels();
				refreshDiskSpace();
			} else if (kind === "error") {
				setDownloadProgress((prev) => {
					const next = { ...prev };
					delete next[modelId];
					return next;
				});
				openSnackbar(false, `Download failed: ${message}`);
			} else if (kind === "load-error") {
				// the download itself succeeded; activating the model failed
				openSnackbar(false, `Model downloaded, but activating it failed: ${message}`);
				refreshModels();
				refreshDiskSpace();
			}
		});
		return () => {
			void unlisten.then((fn) => fn());
		};
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, []);

	// the per-row actions keep stable identities for the memoised ModelRow
	const download = useCallback(
		(modelId: string): void => {
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
	// The stored value is never round-tripped through the UI.
	const saveSecret = (key: string, value: string): void => {
		if (!settings) return;
		// Send ONLY the secret: the Rust command applies present keys, and
		// the spread form would persist whatever is sitting half-typed in
		// the endpoint inputs right now.
		saveAiSettingsApi({ [key]: value })
			.then(() => {
				reloadSavedSettings();
				openSnackbar(true, value === "" ? "Removed" : "Saved");
			})
			.catch((e) => openSnackbar(false, normalizeApiError(e)));
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
			deleteModelApi(modelId)
				.then(() => {
					refreshModels();
					refreshDiskSpace();
				})
				.catch((e) => openSnackbar(false, normalizeApiError(e)));
		},
		[openSnackbar, refreshModels, refreshDiskSpace]
	);

	const cancelDownload = useCallback(
		(modelId: string): void => {
			cancelDownloadApi(modelId).catch((e) => openSnackbar(false, normalizeApiError(e)));
		},
		[openSnackbar]
	);

	const activateModel = useCallback(
		(modelId: string): void => {
			activateModelApi(modelId)
				.then(refreshModels)
				.catch((e) => openSnackbar(false, normalizeApiError(e)));
		},
		[openSnackbar, refreshModels]
	);

	return {
		// state
		models,
		settings,
		setSettings,
		savedSettings,
		setSavedSettings,
		runtime,
		downloadProgress,
		appleStt,
		freeBytes,
		// actions
		refreshModels,
		download,
		save,
		saveSecret,
		saveEndpoint,
		deleteModel,
		cancelDownload,
		activateModel
	};
}
