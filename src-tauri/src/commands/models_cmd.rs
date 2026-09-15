use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;
use tauri::ipc::Channel;
use tauri::{Emitter, Manager, State};

use crate::models::{download_model_file, find_model, model_url, AiSettings, ModelKind};
use crate::types::ModelStatus;
use crate::AppState;

#[tauri::command]
pub fn list_models(state: State<'_, AppState>) -> serde_json::Value {
	let settings = AiSettings::load(&state.db);
	let build = |kind: ModelKind| -> Vec<ModelStatus> {
		let list: &[crate::models::ModelSpec] = match kind {
			ModelKind::Llm => &crate::models::LLM_MODELS,
			ModelKind::Stt => &crate::models::STT_MODELS,
		};
		let progress = state
			.download_progress
			.lock()
			.unwrap_or_else(|e| e.into_inner());
		list.iter()
			.map(|spec| ModelStatus {
				id: spec.id.to_string(),
				label: spec.label.to_string(),
				description: spec.description.to_string(),
				kind: match kind {
					ModelKind::Llm => "llm",
					ModelKind::Stt => "stt",
				}
				.to_string(),
				size_bytes: spec.size_bytes,
				downloaded: state.is_model_downloaded(spec),
				active: match kind {
					ModelKind::Llm => settings.llm_mode == "local" && settings.llm_model == spec.id,
					ModelKind::Stt => settings.stt_model == spec.id,
				},
				downloading: progress.contains_key(spec.id),
				progress: progress.get(spec.id).copied(),
				filename: Some(spec.filename.to_string()),
			})
			.collect()
	};

	json!({
		"llm": build(ModelKind::Llm),
		"stt": build(ModelKind::Stt),
	})
}

#[tauri::command]
pub fn get_runtime_status(state: State<'_, AppState>) -> serde_json::Value {
	json!({
		"llm": state.llm_status.lock().unwrap_or_else(|e| e.into_inner()).clone(),
		"stt": state.stt_status.lock().unwrap_or_else(|e| e.into_inner()).clone(),
	})
}

/// Removes the download's bookkeeping entries when dropped, so even a
/// panicking task can't wedge future downloads with a stale entry.
struct DownloadGuard {
	app: tauri::AppHandle,
	model_id: String,
}

impl Drop for DownloadGuard {
	fn drop(&mut self) {
		if let Some(state) = self.app.try_state::<AppState>() {
			state
				.download_progress
				.lock()
				.unwrap_or_else(|e| e.into_inner())
				.remove(&self.model_id);
			state
				.download_cancels
				.lock()
				.unwrap_or_else(|e| e.into_inner())
				.remove(&self.model_id);
		}
	}
}

#[tauri::command]
pub async fn download_model(
	app: tauri::AppHandle,
	state: State<'_, AppState>,
	model_id: String,
	on_event: Channel<serde_json::Value>,
) -> Result<(), String> {
	let spec = find_model(&model_id, ModelKind::Llm)
		.or_else(|| find_model(&model_id, ModelKind::Stt))
		.ok_or_else(|| format!("unknown model {model_id}"))?
		.clone();

	{
		let mut progress = state
			.download_progress
			.lock()
			.unwrap_or_else(|e| e.into_inner());
		if progress.contains_key(&model_id) {
			return Err("model is already downloading".into());
		}
		progress.insert(model_id.clone(), 0.0);
	}
	// This download gets its own cancel token, fully independent of the
	// generation token - chatting must never kill a download and vice versa.
	let cancel = Arc::new(AtomicBool::new(false));
	state
		.download_cancels
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.insert(model_id.clone(), cancel.clone());

	std::fs::create_dir_all(state.models_dir()).map_err(|e| e.to_string())?;

	let app_handle = app.clone();
	let dest = state.model_path(&spec);
	let url = model_url(&spec);
	let hf_token = AiSettings::load(&state.db).hf_token;

	tauri::async_runtime::spawn(async move {
		let _guard = DownloadGuard {
			app: app_handle.clone(),
			model_id: model_id.clone(),
		};
		{
			let state = app_handle.state::<AppState>();
			let model_id = model_id.clone();
			let mut on_progress = |pct: f64| {
				state
					.download_progress
					.lock()
					.unwrap_or_else(|e| e.into_inner())
					.insert(model_id.clone(), pct);
				let _ = on_event.send(json!({ "kind": "progress", "pct": pct }));
				let _ = app_handle.emit(
					"model-download",
					json!({ "modelId": model_id, "kind": "progress", "pct": pct }),
				);
			};
			let result = download_model_file(
				&url,
				&dest,
				spec.size_bytes,
				spec.sha256,
				&hf_token,
				&cancel,
				&mut on_progress,
			)
			.await;

			match result {
				Ok(()) => {
					let _ = on_event.send(json!({ "kind": "done" }));
					let _ = app_handle.emit(
						"model-download",
						json!({ "modelId": model_id, "kind": "done" }),
					);
					// If this model is the active one, load it right away.
					let state = app_handle.state::<AppState>();
					let settings = AiSettings::load(&state.db);
					let is_active_llm = spec.kind == ModelKind::Llm
						&& settings.llm_mode == "local"
						&& settings.llm_model == spec.id;
					let is_active_stt =
						spec.kind == ModelKind::Stt && settings.stt_model == spec.id;
					if is_active_llm || is_active_stt {
						let app2 = app_handle.clone();
						tauri::async_runtime::spawn_blocking(move || {
							let state = app2.state::<AppState>();
							let result = match spec.kind {
								ModelKind::Llm => state.load_llm(&app2, &spec),
								ModelKind::Stt => state.load_stt(&app2, &spec),
							};
							if let Err(e) = result {
								log::error!("auto-load of {} failed: {e}", spec.id);
							}
						});
					}
				}
				Err(e) => {
					let _ = on_event.send(json!({ "kind": "error", "message": e }));
					let _ = app_handle.emit(
						"model-download",
						json!({ "modelId": model_id, "kind": "error", "message": e }),
					);
				}
			}
		}
	});

	Ok(())
}

#[tauri::command]
pub fn delete_model(
	app: tauri::AppHandle,
	state: State<'_, AppState>,
	model_id: String,
) -> Result<(), String> {
	let spec = find_model(&model_id, ModelKind::Llm)
		.or_else(|| find_model(&model_id, ModelKind::Stt))
		.ok_or_else(|| format!("unknown model {model_id}"))?;
	{
		let progress = state
			.download_progress
			.lock()
			.unwrap_or_else(|e| e.into_inner());
		if progress.contains_key(&model_id) {
			return Err("model is currently downloading".into());
		}
	}
	// A load of this model running in the background would re-install the
	// engine right after deletion; refuse until it finishes.
	match spec.kind {
		ModelKind::Llm => {
			if state.llm_loading.load(Ordering::SeqCst) {
				return Err("model is currently loading - try again in a moment".into());
			}
		}
		ModelKind::Stt => {
			if state.stt_loading.load(Ordering::SeqCst) {
				return Err("model is currently loading - try again in a moment".into());
			}
		}
	}
	let path = state.model_path(spec);
	// Unload the engine BEFORE deleting the file: the loaded engine mmaps
	// the model, and on Windows an open mmap makes remove_file fail.
	let mut runtime = state.runtime.lock().unwrap_or_else(|e| e.into_inner());
	let llm_gone =
		runtime.llm.as_ref().map(|e| e.model_id.clone()).as_deref() == Some(model_id.as_str());
	let stt_gone =
		runtime.stt.as_ref().map(|e| e.model_id.clone()).as_deref() == Some(model_id.as_str());
	if llm_gone {
		runtime.llm = None;
	}
	if stt_gone {
		runtime.stt = None;
	}
	drop(runtime);

	if path.exists() {
		std::fs::remove_file(&path).map_err(|e| e.to_string())?;
	}

	if llm_gone {
		*state.llm_status.lock().unwrap_or_else(|e| e.into_inner()) =
			crate::models::EngineStatus::new("missing", None, None);
		state.emit_llm_status(&app);
	}
	if stt_gone {
		*state.stt_status.lock().unwrap_or_else(|e| e.into_inner()) =
			crate::models::EngineStatus::new("missing", None, None);
		state.emit_stt_status(&app);
	}
	Ok(())
}

/// Cancel an in-flight download. The token is checked between chunks, so
/// the transfer stops within a second or two and the `.part` file is
/// removed by the downloader's normal failure path.
#[tauri::command]
pub fn cancel_download(state: State<'_, AppState>, model_id: String) -> Result<(), String> {
	let token = state
		.download_cancels
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.get(&model_id)
		.cloned();
	match token {
		Some(token) => {
			token.store(true, Ordering::Relaxed);
			Ok(())
		}
		None => Err(format!("no download in progress for {model_id}")),
	}
}

/// Explicitly activate (and load if needed) a downloaded model. The
/// settings row is only updated once the engine actually loaded, so the
/// recorded active model can never disagree with the runtime.
#[tauri::command]
pub async fn activate_model(
	app: tauri::AppHandle,
	state: State<'_, AppState>,
	model_id: String,
) -> Result<(), String> {
	let spec = find_model(&model_id, ModelKind::Llm)
		.or_else(|| find_model(&model_id, ModelKind::Stt))
		.ok_or_else(|| format!("unknown model {model_id}"))?
		.clone();
	if !state.is_model_downloaded(&spec) {
		return Err("model is not downloaded".into());
	}

	let app_handle = app.clone();
	tauri::async_runtime::spawn_blocking(move || {
		let state = app_handle.state::<AppState>();
		let result = match spec.kind {
			ModelKind::Llm => state.load_llm(&app_handle, &spec),
			ModelKind::Stt => state.load_stt(&app_handle, &spec),
		};
		match result {
			Ok(()) => {
				let mut settings = AiSettings::load(&state.db);
				match spec.kind {
					ModelKind::Llm => {
						settings.llm_mode = "local".into();
						settings.llm_model = spec.id.to_string();
					}
					ModelKind::Stt => {
						settings.stt_model = spec.id.to_string();
					}
				}
				settings.save(&state.db);
			}
			Err(e) => {
				// The status event already carries the error to the UI;
				// this keeps it in the (now real) log file too.
				log::error!("activate_model({}) failed: {e}", spec.id);
			}
		}
	});
	Ok(())
}
