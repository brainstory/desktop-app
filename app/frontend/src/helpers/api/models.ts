import { invoke, Channel } from "@tauri-apps/api/core";

export interface ModelStatus {
	id: string;
	label: string;
	description: string;
	kind: "llm" | "stt";
	sizeBytes: number;
	downloaded: boolean;
	active: boolean;
	downloading: boolean;
	/** download progress 0-100; negative = total size unknown */
	progress?: number;
	filename?: string;
}

export interface ModelsResponse {
	llm: ModelStatus[];
	stt: ModelStatus[];
}

/** EngineStatus from the backend (snake_case model_id/error keys). */
export interface EngineStatus {
	state: "ready" | "loading" | "missing" | "error" | "external";
	model_id?: string;
	error?: string;
}

export interface AiSettingsResponse {
	llmMode: string;
	llmModel: string;
	sttModel: string;
	sttEngine: string;
	sttLanguage: string;
	hfTokenSet: boolean;
	hfTokenHint: string | null;
	extLlmBaseUrl: string;
	extLlmApiKeySet: boolean;
	extLlmApiKeyHint: string | null;
	extLlmModel: string;
	extSttBaseUrl: string;
	extSttApiKeySet: boolean;
	extSttApiKeyHint: string | null;
	extSttModel: string;
}

/** Built-in Apple Speech engine (macOS 26+) capability + locale support. */
export interface AppleSttStatus {
	available: boolean;
	authorized: boolean;
	supportedLocales: string[];
	installedLocales: string[];
}

/** List the known local models with download/active status */
export function listModelsApi(): Promise<ModelsResponse> {
	return invoke("list_models");
}

/** Get the status of the local inference engines (llm + stt) */
export function getRuntimeStatusApi(): Promise<{ llm: EngineStatus; stt: EngineStatus }> {
	return invoke("get_runtime_status");
}

/** Availability + locale support of the built-in Apple Speech engine */
export function getAppleSttStatusApi(): Promise<AppleSttStatus> {
	return invoke("get_apple_stt_status");
}

export interface DownloadEvent {
	kind: "progress" | "done" | "error";
	pct?: number;
	message?: string;
}

/**
 * Download a model. Progress events arrive on the returned channel:
 * {kind: "progress", pct} | {kind: "done"} | {kind: "error", message}
 */
export function downloadModelApi(modelId: string): {
	channel: Channel<DownloadEvent>;
	invokePromise: Promise<void>;
} {
	const channel = new Channel<DownloadEvent>();
	const invokePromise = invoke<void>("download_model", {
		modelId,
		onEvent: channel
	});
	return { channel, invokePromise };
}

export async function deleteModelApi(modelId: string): Promise<void> {
	await invoke("delete_model", { modelId });
}

/** Cancel an in-flight download; the backend removes its .part file. */
export async function cancelDownloadApi(modelId: string): Promise<void> {
	return invoke("cancel_download", { modelId });
}

/** Activate a downloaded model (sets it active and loads it) */
export async function activateModelApi(modelId: string): Promise<void> {
	await invoke("activate_model", { modelId });
}

export function getAiSettingsApi(): Promise<AiSettingsResponse> {
	return invoke("get_ai_settings");
}

export async function saveAiSettingsApi(ai: object): Promise<void> {
	return invoke("save_ai_settings", { ai });
}

export function testLlmEndpointApi(): Promise<string> {
	return invoke("test_llm_endpoint");
}

export function testSttEndpointApi(): Promise<string> {
	return invoke("test_stt_endpoint");
}

export function sendTestNotificationApi(): Promise<void> {
	return invoke("send_test_notification");
}
