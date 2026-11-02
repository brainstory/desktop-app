import { Channel } from "@tauri-apps/api/core";
import { invokeCommand } from "@src/tauri/invoke";
import type { AiSettingsResponse } from "@src/tauri/generated/AiSettingsResponse";
import type { DownloadEvent as ModelDownloadEvent } from "@src/tauri/generated/DownloadEvent";
import type { EngineStatus } from "@src/tauri/generated/EngineStatus";
import type { ModelStatus } from "@src/tauri/generated/ModelStatus";

export type { AiSettingsResponse, EngineStatus, ModelStatus };

export interface ModelsResponse {
	llm: ModelStatus[];
	stt: ModelStatus[];
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

/**
 * Download a model. Progress events arrive on the returned channel and
 * are also broadcast app-wide (see the $downloads store), which drives
 * the UI: progress / paused (with where it stopped) / done / cancelled /
 * error / load-error (download succeeded but activating the model
 * failed). A user cancel is not an error.
 */
export function downloadModelApi(modelId: string): {
	channel: Channel<ModelDownloadEvent>;
	invokePromise: Promise<void>;
} {
	const channel = new Channel<ModelDownloadEvent>();
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
