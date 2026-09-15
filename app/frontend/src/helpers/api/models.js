import { invoke, Channel } from "@tauri-apps/api/core";

/** List the known local models with download/active status */
export async function listModelsApi() {
	return invoke("list_models");
}

/** Get the status of the local inference engines (llm + stt) */
export async function getRuntimeStatusApi() {
	return invoke("get_runtime_status");
}

/**
 * Download a model. Progress events arrive on the returned channel:
 * {kind: "progress", pct} | {kind: "done"} | {kind: "error", message}
 */
export function downloadModelApi(modelId) {
	const channel = new Channel();
	const invokePromise = invoke("download_model", {
		modelId,
		onEvent: channel
	});
	return { channel, invokePromise };
}

export async function deleteModelApi(modelId) {
	return invoke("delete_model", { modelId });
}

/** Cancel an in-flight download; the backend removes its .part file. */
export async function cancelDownloadApi(modelId) {
	return invoke("cancel_download", { modelId });
}

/** Activate a downloaded model (sets it active and loads it) */
export async function activateModelApi(modelId) {
	return invoke("activate_model", { modelId });
}

export async function getAiSettingsApi() {
	return invoke("get_ai_settings");
}

export async function saveAiSettingsApi(ai) {
	return invoke("save_ai_settings", { ai });
}

export async function testLlmEndpointApi() {
	return invoke("test_llm_endpoint");
}

export async function testSttEndpointApi() {
	return invoke("test_stt_endpoint");
}

export async function sendTestNotificationApi() {
	return invoke("send_test_notification");
}
