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
		let progress = state.download_progress.lock().unwrap();
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
		"llm": state.llm_status.lock().unwrap().clone(),
		"stt": state.stt_status.lock().unwrap().clone(),
	})
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
		let mut progress = state.download_progress.lock().unwrap();
		if progress.contains_key(&model_id) {
			return Err("model is already downloading".into());
		}
		progress.insert(model_id.clone(), 0.0);
	}

	std::fs::create_dir_all(state.models_dir()).map_err(|e| e.to_string())?;

	let cancel = state.generation_cancel.lock().unwrap().clone();
	let app_handle = app.clone();
	let dest = state.model_path(&spec);
	let url = model_url(&spec);

	tauri::async_runtime::spawn(async move {
		{
			let state = app_handle.state::<AppState>();
			let mut on_progress = |pct: f64| {
				state.download_progress.lock().unwrap().insert(model_id.clone(), pct);
				let _ = on_event.send(json!({ "kind": "progress", "pct": pct }));
				let _ = app_handle.emit(
					"model-download",
					json!({ "modelId": model_id, "kind": "progress", "pct": pct }),
				);
			};
			let result = download_model_file(&url, &dest, &cancel, &mut on_progress).await;

			let state = app_handle.state::<AppState>();
			state.download_progress.lock().unwrap().remove(&model_id);

			match result {
				Ok(()) => {
					let _ = on_event.send(json!({ "kind": "done" }));
					let _ = app_handle.emit(
						"model-download",
						json!({ "modelId": model_id, "kind": "done" }),
					);
					// If this model is the active one, load it right away.
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
							let spec = find_model(&model_id, ModelKind::Llm)
								.or_else(|| find_model(&model_id, ModelKind::Stt))
								.unwrap();
							match spec.kind {
								ModelKind::Llm => state.load_llm(&app2, spec),
								ModelKind::Stt => state.load_stt(&app2, spec),
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
		let progress = state.download_progress.lock().unwrap();
		if progress.contains_key(&model_id) {
			return Err("model is currently downloading".into());
		}
	}
	let path = state.model_path(spec);
	if path.exists() {
		std::fs::remove_file(&path).map_err(|e| e.to_string())?;
	}

	// If the deleted model is loaded, unload it so the UI reflects reality.
	let mut runtime = state.runtime.lock().unwrap();
	let llm_gone = runtime.llm.as_ref().map(|e| e.model_id.clone()).as_deref() == Some(model_id.as_str());
	let stt_gone = runtime.stt.as_ref().map(|e| e.model_id.clone()).as_deref() == Some(model_id.as_str());
	if llm_gone {
		runtime.llm = None;
	}
	if stt_gone {
		runtime.stt = None;
	}
	drop(runtime);
	if llm_gone {
		*state.llm_status.lock().unwrap() =
			crate::models::EngineStatus::new("missing", None, None);
		state.emit_llm_status(&app);
	}
	if stt_gone {
		*state.stt_status.lock().unwrap() =
			crate::models::EngineStatus::new("missing", None, None);
		state.emit_stt_status(&app);
	}
	Ok(())
}

/// Explicitly activate (and load if needed) a downloaded model.
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

	let app_handle = app.clone();
	tauri::async_runtime::spawn_blocking(move || {
		let state = app_handle.state::<AppState>();
		match spec.kind {
			ModelKind::Llm => state.load_llm(&app_handle, &spec),
			ModelKind::Stt => state.load_stt(&app_handle, &spec),
		}
	});
	Ok(())
}
