import { Channel } from "@tauri-apps/api/core";
import { invokeCommand } from "@src/tauri/invoke";

export interface ModelStatus {
	id: string;
	label: string;
	description: string;
	kind: "llm" | "stt";
	sizeBytes: number;
	downloaded: boolean;
	active: boolean;
	downloading: boolean;
	/** a user pause stopped the transfer; `progress` says where */
	paused?: boolean;
	/** download progress 0-100; negative = total size unknown */
	progress?: number;
	filename?: string;
	/**
	 * Approximate KV-cache bytes per token for local LLM rows (absent for
	 * STT): window tokens × this ≈ extra memory for a context window.
	 */
	kvBytesPerToken?: number;
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
	/** Requested local context window: 0 = app default (16k); else a
	 * multiple of 1024 in 8192..=131072. Changing it reloads the model. */
	llmCtxTokens: number;
	sttModel: string;
	/** "local" or "external": whether transcription uses the external endpoint */
	sttMode: string;
	sttEngine: string;
	sttLanguage: string;
	hfTokenSet: boolean;
	hfTokenHint: string | null;
	/** true when at least one stored secret lives in the plaintext DB
	 * fallback instead of the OS keychain */
	secretsPlaintext: boolean;
	/** Download endpoint override (mirror); empty = env/default */
	hfEndpoint: string;
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
	return invokeCommand("listModels");
}

/** Get the status of the local inference engines (llm + stt) */
export function getRuntimeStatusApi(): Promise<{ llm: EngineStatus; stt: EngineStatus }> {
	return invokeCommand("getRuntimeStatus");
}

/** Availability + locale support of the built-in Apple Speech engine */
export function getAppleSttStatusApi(): Promise<AppleSttStatus> {
	return invokeCommand("getAppleSttStatus");
}

/** Free bytes on the volume holding the models directory (warn-only UI) */
export function getFreeDiskSpaceApi(): Promise<number> {
	return invokeCommand("getFreeDiskSpace");
}

export interface DownloadEvent {
	kind: "progress" | "paused" | "done" | "cancelled" | "error" | "load-error";
	pct?: number;
	bytesPerSec?: number;
	etaSecs?: number;
	message?: string;
}

/**
 * Download a model. Progress events arrive on the returned channel:
 * {kind: "progress", pct} | {kind: "done"} | {kind: "cancelled"} |
 * {kind: "error", message} | {kind: "load-error", message} (download
 * succeeded but activating the model failed). A user cancel is not an
 * error. The same events are also broadcast app-wide (see the
 * $downloads store), which drives the UI.
 */
export function downloadModelApi(modelId: string): {
	channel: Channel<DownloadEvent>;
	invokePromise: Promise<void>;
} {
	const channel = new Channel<DownloadEvent>();
	const invokePromise = invokeCommand("downloadModel", {
		modelId,
		onEvent: channel
	});
	return { channel, invokePromise };
}

export async function deleteModelApi(modelId: string): Promise<void> {
	await invokeCommand("deleteModel", { modelId });
}

/** Cancel an in-flight download; the backend removes its .part file. */
export async function cancelDownloadApi(modelId: string): Promise<void> {
	return invokeCommand("cancelDownload", { modelId });
}

/**
 * Pause an in-flight download: the transfer stops keeping its staged
 * `.part`, and starting the same download again resumes from it.
 */
export async function pauseDownloadApi(modelId: string): Promise<void> {
	return invokeCommand("pauseDownload", { modelId });
}

/** Activate a downloaded model (sets it active and loads it) */
export async function activateModelApi(modelId: string): Promise<void> {
	await invokeCommand("activateModel", { modelId });
}

export function getAiSettingsApi(): Promise<AiSettingsResponse> {
	return invokeCommand("getAiSettings");
}

export async function saveAiSettingsApi(ai: object): Promise<void> {
	return invokeCommand("saveAiSettings", { ai });
}

export function testLlmEndpointApi(apiKey?: string): Promise<string> {
	// a typed candidate key is used for the test instead of the stored
	// one; empty/whitespace counts as "use the stored key"
	return invokeCommand("testLlmEndpoint", apiKey?.trim() ? { apiKey } : undefined);
}

export function testSttEndpointApi(apiKey?: string): Promise<string> {
	return invokeCommand("testSttEndpoint", apiKey?.trim() ? { apiKey } : undefined);
}
