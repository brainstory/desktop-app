use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;
use tauri::ipc::Channel;
use tauri::{Emitter, Manager, State};

use crate::models::{download_model_file, find_model, model_url, AiSettings, ModelKind};
use crate::types::ModelStatus;
use crate::AppState;

#[tauri::command]
pub async fn list_models(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
	// async command body runs on the runtime, and the blocking work
	// (DB reads, progress-lock scans, file-existence checks for every
	// catalog entry) stays off it - and off the main thread, where a
	// sync command would run it.
	tauri::async_runtime::spawn_blocking(move || {
		let state = app.state::<AppState>();
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
						ModelKind::Llm => {
							!settings.uses_external_llm() && settings.llm_model == spec.id
						}
						// A whisper model is only "active" when whisper actually
						// handles local transcription (Apple Speech mode demotes it
						// to fallback).
						ModelKind::Stt => {
							settings.stt_model == spec.id
								&& settings.effective_stt_engine()
									== crate::models::SpeechEngine::Whisper
						}
					},
					downloading: progress.contains_key(spec.id),
					progress: progress.get(spec.id).copied(),
					filename: Some(spec.filename.to_string()),
				})
				.collect()
		};

		Ok(json!({
			"llm": build(ModelKind::Llm),
			"stt": build(ModelKind::Stt),
		}))
	})
	.await
	.map_err(|e| format!("list models task failed: {e}"))?
}

#[tauri::command]
pub async fn get_runtime_status(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
	tauri::async_runtime::spawn_blocking(move || {
		let state = app.state::<AppState>();
		Ok(json!({
			"llm": state.llm_status.lock().unwrap_or_else(|e| e.into_inner()).clone(),
			"stt": state.stt_status.lock().unwrap_or_else(|e| e.into_inner()).clone(),
		}))
	})
	.await
	.map_err(|e| format!("runtime status task failed: {e}"))?
}

/// Availability and locale support of the built-in Apple Speech engine
/// (macOS 26+). Powers the engine selector and language picker in settings.
#[tauri::command]
pub async fn get_apple_stt_status() -> Result<serde_json::Value, String> {
	// the Swift bridge blocks (locale enumeration, permission state)
	tauri::async_runtime::spawn_blocking(|| {
		let status = crate::stt_apple::status();
		Ok(json!({
			"available": status.available,
			"authorized": status.authorized,
			"supportedLocales": status.supported_locales,
			"installedLocales": status.installed_locales,
		}))
	})
	.await
	.map_err(|e| format!("apple stt status task failed: {e}"))?
}

/// Free space (bytes) on the volume holding the models directory, so the
/// UI can warn before a multi-GB download. Warn-only: an actual download
/// that runs out of space still fails cleanly through the normal error
/// path (hash/size verification catches the truncated file).
#[tauri::command]
pub async fn get_free_disk_space(state: State<'_, AppState>) -> Result<u64, String> {
	let models_dir = state.models_dir();
	tauri::async_runtime::spawn_blocking(move || {
		fs4::available_space(&models_dir).map_err(|e| e.to_string())
	})
	.await
	.map_err(|e| format!("disk space task failed: {e}"))?
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
			unregister_download(&state, &self.model_id);
		}
	}
}

/// Insert a download into both bookkeeping maps under one critical
/// section, so a concurrent `cancel_download` or `download_model` can
/// never observe a half-registered download. Err means this model is
/// already downloading.
fn register_download(state: &AppState, model_id: &str) -> Result<Arc<AtomicBool>, String> {
	// Lock ordering (progress, then cancels) must match unregister_download.
	let mut progress = state
		.download_progress
		.lock()
		.unwrap_or_else(|e| e.into_inner());
	let mut cancels = state
		.download_cancels
		.lock()
		.unwrap_or_else(|e| e.into_inner());
	if progress.contains_key(model_id) {
		return Err("model is already downloading".into());
	}
	let cancel = Arc::new(AtomicBool::new(false));
	progress.insert(model_id.to_string(), 0.0);
	cancels.insert(model_id.to_string(), cancel.clone());
	Ok(cancel)
}

/// Remove a download's bookkeeping entries (inverse of register_download).
fn unregister_download(state: &AppState, model_id: &str) {
	state
		.download_progress
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.remove(model_id);
	state
		.download_cancels
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.remove(model_id);
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

	// All fallible setup happens BEFORE the bookkeeping is registered: a
	// failure here must not leave the model reported as "downloading"
	// forever (which would block re-download and delete until restart).
	std::fs::create_dir_all(state.models_dir()).map_err(|e| e.to_string())?;

	// This download gets its own cancel token, fully independent of the
	// generation token - chatting must never kill a download and vice versa.
	let cancel = register_download(&state, &model_id)?;
	// The guard exists before the spawn, so nothing between registration
	// and the spawned task can leak the bookkeeping entries.
	let guard = DownloadGuard {
		app: app.clone(),
		model_id: model_id.clone(),
	};

	let app_handle = app.clone();
	let dest = state.model_path(&spec);
	let url = model_url(&spec);
	let hf_token = AiSettings::load(&state.db).hf_token;

	tauri::async_runtime::spawn(async move {
		let _guard = guard;
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
					// The download itself is finished: release the
					// bookkeeping before the auto-load below, so the
					// model stops reporting "downloading" while its
					// engine spins up.
					drop(_guard);
					// If this model is the active one, load it right away.
					let state = app_handle.state::<AppState>();
					let settings = AiSettings::load(&state.db);
					let is_active_llm = spec.kind == ModelKind::Llm
						&& !settings.uses_external_llm()
						&& settings.llm_model == spec.id;
					let is_active_stt =
						spec.kind == ModelKind::Stt && settings.stt_model == spec.id;
					if is_active_llm || is_active_stt {
						// A load failure must not read as a successful
						// activation (the runtime-status event fires from
						// inside the loader too); report it on the same
						// channel/event the download UI already listens to.
						let app2 = app_handle.clone();
						let spec_id = spec.id;
						let load_result = tauri::async_runtime::spawn_blocking(move || {
							let state = app2.state::<AppState>();
							match spec.kind {
								ModelKind::Llm => state.load_llm(&app2, &spec),
								ModelKind::Stt => state.load_stt(&app2, &spec),
							}
						})
						.await
						.unwrap_or_else(|e| Err(format!("load task failed: {e}")));
						if let Err(e) = load_result {
							log::error!("auto-load of {spec_id} failed: {e}");
							let _ = on_event.send(json!({ "kind": "load-error", "message": e }));
							let _ = app_handle.emit(
								"model-download",
								json!({
									"modelId": model_id,
									"kind": "load-error",
									"message": e
								}),
							);
						}
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
pub async fn delete_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
	// Dropping the mmapped engine and removing a multi-GB file can stall
	// for seconds; never do that on the main thread (a sync command).
	tauri::async_runtime::spawn_blocking(move || {
		let state = app.state::<AppState>();
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
	})
	.await
	.map_err(|e| format!("delete task failed: {e}"))?
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
	// Await the load so the invoke result tells the caller whether the
	// engine actually activated: load_llm/load_stt refuse concurrent
	// loads ("a model is already loading") and fail on bad files, and
	// fire-and-forgetting made the UI believe activation succeeded.
	let outcome = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
		let state = app_handle.state::<AppState>();
		match match spec.kind {
			ModelKind::Llm => state.load_llm(&app_handle, &spec),
			ModelKind::Stt => state.load_stt(&app_handle, &spec),
		} {
			Ok(()) => {
				let mut settings = AiSettings::load(&state.db);
				match spec.kind {
					ModelKind::Llm => {
						settings.llm_mode = crate::models::LlmMode::Local;
						settings.llm_model = spec.id.to_string();
					}
					ModelKind::Stt => {
						settings.stt_model = spec.id.to_string();
					}
				}
				// the settings row is only updated once the engine
				// actually loaded, so the recorded active model can never
				// disagree with the runtime
				settings.save(&state.db)
			}
			Err(e) => Err(e),
		}
	})
	.await
	.map_err(|e| format!("activation task failed: {e}"))?;
	outcome
}

#[cfg(test)]
mod tests {
	use super::{register_download, unregister_download};
	use crate::AppState;
	use std::sync::atomic::Ordering;

	fn temp_state(name: &str) -> AppState {
		let dir = std::env::temp_dir().join(format!(
			"brainstory-dl-bookkeeping-{name}-{}",
			uuid::Uuid::new_v4()
		));
		std::fs::create_dir_all(&dir).expect("make temp dir");
		let db = crate::db::Db::open(&dir.join("test.db")).expect("open test db");
		AppState::new(db, dir)
	}

	/// A download that ends (or never really starts) must release its slot,
	/// or the model is stuck as "downloading" until app restart.
	#[test]
	fn registration_is_atomic_and_cleanup_releases_the_slot() {
		let state = temp_state("slot");
		let cancel = register_download(&state, "m").expect("first registration wins");
		cancel.store(true, Ordering::Relaxed);

		// both maps carry the entry, sharing the same cancel token
		assert!(state
			.download_progress
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.contains_key("m"));
		let token = state
			.download_cancels
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.get("m")
			.cloned()
			.expect("cancel token registered alongside progress");
		assert!(token.load(Ordering::Relaxed));

		// a second registration while the first holds the slot is refused
		assert!(register_download(&state, "m").is_err());

		// once the guard's cleanup runs, the slot is reusable
		unregister_download(&state, "m");
		assert!(!state
			.download_progress
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.contains_key("m"));
		assert!(state
			.download_cancels
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.get("m")
			.is_none());
		assert!(register_download(&state, "m").is_ok());
	}
}
