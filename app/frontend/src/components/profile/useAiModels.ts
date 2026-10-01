/**
 * Data hook for the AI Models card: all state (catalog, settings,
 * runtime status, download progress, Apple STT availability, free disk
 * space), the event listeners, and the action callbacks. Extracted from
 * the 800-line card so the component is rendering + composition.
 */

import { useEffect, useId, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
	activateModelApi,
	cancelDownloadApi,
	downloadModelApi,
	getAiSettingsApi,
	getAppleSttStatusApi,
	getFreeDiskSpaceApi,
	getRuntimeStatusApi,
	listModelsApi,
	saveAiSettingsApi,
	deleteModelApi,
	type AiSettingsResponse,
	type AppleSttStatus,
	type EngineStatus,
	type ModelsResponse
} from "@helpers/api/models";
import { normalizeApiError } from "@helpers/helpers";
import { useTimeout } from "@src/hooks/useTimeout";

export function useAiModels(openSnackbar: (isSuccess: boolean, message: string) => void) {
	const [models, setModels] = useState<ModelsResponse>({ llm: [], stt: [] });
	const [settings, setSettings] = useState<AiSettingsResponse | null>(null);
	/** last persisted snapshot of the endpoint fields, for dirty tracking */
	const [savedSettings, setSavedSettings] = useState<AiSettingsResponse | null>(null);
	const [runtime, setRuntime] = useState<{
		llm: Partial<EngineStatus>;
		stt: Partial<EngineStatus>;
	}>({ llm: {}, stt: {} });
	const [downloadProgress, setDownloadProgress] = useState<Record<string, number>>({});
	const [appleStt, setAppleStt] = useState<AppleSttStatus | null>(null);
	const [freeBytes, setFreeBytes] = useState<number | null>(null);
	/** model id awaiting a second "really delete?" click */
	const [confirmingDeleteId, setConfirmingDeleteId] = useState<string | null>(null);
	const [scheduleDeleteReset] = useTimeout();
	const externalLlmLabelId = useId();

	const refresh = () => {
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
		getRuntimeStatusApi()
			.then(setRuntime)
			.catch((e) => console.error("status failed", e));
		getAppleSttStatusApi()
			.then(setAppleStt)
			.catch((e) => console.error("apple stt status failed", e));
		// re-checked on every refresh: downloads and deletions change it
		getFreeDiskSpaceApi()
			.then(setFreeBytes)
			.catch((e) => console.error("free disk space failed", e));
	};

	useEffect(() => {
		getAiSettingsApi()
			.then((data) => {
				setSettings(data);
				setSavedSettings(data);
			})
			.catch((e) => console.error("ai settings failed", e));
		refresh();

		const unlisteners = [
			listen("llm-status", refresh),
			listen("stt-status", refresh),
			// downloads continue in the backend across page navigation
			listen<{
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
					refresh();
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
					refresh();
				}
			})
		];
		return () => {
			unlisteners.forEach((p) => p.then((fn) => fn()));
		};
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, []);

	const download = (modelId: string): void => {
		downloadModelApi(modelId).invokePromise.catch((e) => {
			// starting a download that's already running is harmless - the
			// progress bar is already driven by backend events
			if (!String(e).includes("already downloading")) {
				openSnackbar(false, normalizeApiError(e));
			}
			refresh();
		});
	};

	const save = (updates: Partial<AiSettingsResponse>): void => {
		if (!settings) return;
		const next = { ...settings, ...updates };
		setSettings(next);
		saveAiSettingsApi(next as Record<string, unknown>)
			.then(() => {
				setSavedSettings(next);
				refresh();
			})
			.catch((e) => openSnackbar(false, normalizeApiError(e)));
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
				refresh();
				// re-sync only the saved snapshot (the dirty flag's
				// reference): `settings` keeps the live form state
				getAiSettingsApi()
					.then((saved) => setSavedSettings(saved))
					.catch((e) => console.error("failed to reload ai settings", e));
				openSnackbar(true, value === "" ? "Removed" : "Saved");
			})
			.catch((e) => openSnackbar(false, normalizeApiError(e)));
	};

	/** Mirror endpoint row sends ONLY its field (like the secret rows). */
	const saveEndpoint = (value: string): void => {
		if (!settings) return;
		saveAiSettingsApi({ hfEndpoint: value })
			.then(() => {
				refresh();
				getAiSettingsApi()
					.then((saved) => setSavedSettings(saved))
					.catch((e) => console.error("failed to reload ai settings", e));
				openSnackbar(true, value.trim() ? "Mirror saved" : "Mirror cleared");
			})
			.catch((e) => openSnackbar(false, normalizeApiError(e)));
	};

	const deleteModel = (modelId: string): void => {
		deleteModelApi(modelId)
			.then(refresh)
			.catch((e) => openSnackbar(false, normalizeApiError(e)));
	};

	const cancelDownload = (modelId: string): void => {
		cancelDownloadApi(modelId).catch((e) => openSnackbar(false, normalizeApiError(e)));
	};

	const activateModel = (modelId: string): void => {
		activateModelApi(modelId)
			.then(refresh)
			.catch((e) => openSnackbar(false, normalizeApiError(e)));
	};

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
		confirmingDeleteId,
		externalLlmLabelId,
		// actions
		refresh,
		download,
		save,
		saveSecret,
		saveEndpoint,
		deleteModel,
		cancelDownload,
		activateModel,
		setConfirmingDeleteId,
		scheduleDeleteReset
	};
}
