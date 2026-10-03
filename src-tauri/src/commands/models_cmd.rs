use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;
use tauri::ipc::Channel;
use tauri::{Emitter, Manager, State};

use crate::models::{download_model_file, find_model, kv_bytes_per_token, model_url, ModelKind};
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
		let settings = state.ai_settings();
		let build = |kind: ModelKind| -> Vec<serde_json::Value> {
			let list: &[crate::models::ModelSpec] = match kind {
				ModelKind::Llm => &crate::models::LLM_MODELS,
				ModelKind::Stt => &crate::models::STT_MODELS,
			};
			let progress = state
				.download_progress
				.lock()
				.unwrap_or_else(|e| e.into_inner());
			list.iter()
				.map(|spec| {
					let mut row = serde_json::to_value(ModelStatus {
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
					.expect("serializing a ModelStatus row cannot fail");
					// Approximate KV-cache bytes/token powers the
					// context-window memory hint; documented catalog
					// constants for LLM rows, absent for STT rows.
					if let ModelKind::Llm = kind {
						row["kvBytesPerToken"] = json!(kv_bytes_per_token(spec));
					}
					row
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
	// Downloads land in the hub cache now: warn about the volume they
	// will actually hit, not the (legacy) app models dir.
	let _ = state;
	tauri::async_runtime::spawn_blocking(|| {
		let dir = crate::models::primary_hub_cache();
		std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
		fs4::available_space(&dir).map_err(|e| e.to_string())
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
	// a cancel entry without progress is a finished download whose
	// auto-load still holds the slot
	if progress.contains_key(model_id) || cancels.contains_key(model_id) {
		return Err("model is already downloading".into());
	}
	let cancel = Arc::new(AtomicBool::new(false));
	progress.insert(model_id.to_string(), 0.0);
	cancels.insert(model_id.to_string(), cancel.clone());
	Ok(cancel)
}

/// The transfer finished: stop reporting the model as downloading, but
/// keep its cancel entry (which still holds the slot) until the
/// DownloadGuard drops after the auto-load.
fn finish_download_progress(state: &AppState, model_id: &str) {
	state
		.download_progress
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.remove(model_id);
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

	// Already present (app copy or hub cache)? Nothing to download -
	// the UI hides the button, this covers races and stale cards.
	if state.resolve_model_file(&spec).is_some() {
		return Err("model is already downloaded".into());
	}

	// Downloads land directly in the hub cache as content-addressed
	// blobs (blobs/<pinned sha256>), the same layout other HF tooling
	// uses, so one copy serves everyone and nothing migrates later.
	let cache = crate::models::primary_hub_cache();
	let dest = crate::models::hf_blob_path(&cache, &spec);
	// All fallible setup happens BEFORE the bookkeeping is registered: a
	// failure here must not leave the model reported as "downloading"
	// forever (which would block re-download and delete until restart).
	std::fs::create_dir_all(dest.parent().ok_or("blob path has no parent")?)
		.map_err(|e| e.to_string())?;

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
	let url = model_url(&spec, &crate::models::hf_endpoint(&state.ai_settings()));
	let hf_token = state.ai_settings().hf_token;

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
					// publish the blob under snapshots/<sha>/<file> so
					// discovery (ours and other HF tools) sees it
					if let Err(e) = crate::models::materialize_snapshot(&cache, &spec) {
						log::error!("could not create the cache snapshot for {}: {e}", spec.id);
					}
					let _ = on_event.send(json!({ "kind": "done" }));
					let _ = app_handle.emit(
						"model-download",
						json!({ "modelId": model_id, "kind": "done" }),
					);
					// The download itself is finished: the model stops
					// reporting "downloading" while its engine spins up,
					// but the guard (and its cancel entry) lives until the
					// auto-load below is done, so delete_model can't slip
					// in and remove the file the load is about to open.
					finish_download_progress(&app_handle.state::<AppState>(), &model_id);
					// If this model is the active one, load it right away.
					let state = app_handle.state::<AppState>();
					if let Some(generation) =
						download_auto_load_plan(&state, &spec, crate::apple::speech_available())
					{
						// A load failure must not read as a successful
						// activation (the runtime-status event fires from
						// inside the loader too); report it on the same
						// channel/event the download UI already listens to.
						let app2 = app_handle.clone();
						let spec_id = spec.id;
						let load_result = tauri::async_runtime::spawn_blocking(move || {
							let state = app2.state::<AppState>();
							let events = crate::AppStatusEvents(&app2);
							match spec.kind {
								ModelKind::Llm => state.load_llm_events(&events, &spec, generation),
								ModelKind::Stt => state.load_stt_events(&events, &spec, generation),
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

/// Why `spec` cannot be deleted right now, if anything: its download
/// is still running, or just finished and is about to auto-load (the
/// cancel entry holds the slot until that load is done). Download
/// bookkeeping only - a load of the engine kind is NOT checked here:
/// the delete runs under the engine slot claim, which waits out or
/// times out on loads, and the claim itself holds the very flag a
/// loading check would read as busy.
fn delete_blocked(state: &AppState, spec: &crate::models::ModelSpec) -> Option<&'static str> {
	if state
		.download_progress
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.contains_key(spec.id)
	{
		return Some("model is currently downloading");
	}
	if state
		.download_cancels
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.contains_key(spec.id)
	{
		return Some("model is still being set up - try again in a moment");
	}
	None
}

/// How long a delete waits for an in-flight load of the model's engine
/// kind before reporting busy (a load that long is wedged).
const DELETE_SLOT_WAIT: std::time::Duration = std::time::Duration::from_secs(300);

/// Delete `spec`'s files: the legacy app-dir copy first, then every
/// hub-cache candidate (snapshot links, and the blob only when no
/// other snapshot in that repo still references it - huggingface's
/// own pruning rule; anything unverifiable is retained). Returns
/// whether anything was removed.
fn delete_model_files(
	state: &AppState,
	spec: &crate::models::ModelSpec,
	caches: &[std::path::PathBuf],
) -> Result<bool, String> {
	let mut deleted = false;
	let app_path = state.model_path(spec);
	if app_path.is_file() {
		std::fs::remove_file(&app_path).map_err(|e| e.to_string())?;
		deleted = true;
	}
	for cache in caches {
		deleted |= crate::models::remove_cached_model(cache, spec)?;
	}
	Ok(deleted)
}

/// The exclusive delete for `spec` against the real runtime slots,
/// over an injectable event sink and hub-cache list (tests point the
/// caches at temp dirs; production passes every candidate).
fn delete_model_events(
	state: &AppState,
	events: &dyn crate::StatusEvents,
	spec: &crate::models::ModelSpec,
	caches: &[std::path::PathBuf],
	slot_wait: std::time::Duration,
) -> Result<(), String> {
	let delete_files = || delete_model_files(state, spec, caches);
	let blocked = || delete_blocked(state, spec);
	state.delete_model_kind_events(events, spec, &blocked, &delete_files, slot_wait)
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
		let events = crate::AppStatusEvents(&app);
		let caches = crate::models::hf_hub_cache_candidates();
		delete_model_events(&state, &events, spec, &caches, DELETE_SLOT_WAIT)
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

/// Whether a finished download of `spec` should load right now, and
/// the settings generation the decision was made at (None: no load).
/// Routed through the same plan the model loader uses instead of
/// comparing model ids, so the auto-load can never contradict the
/// selected routing: external STT or an explicit Apple Speech selection
/// loads no whisper even when it is the recorded stt model, and
/// external LLM mode loads no local engine. Explicit Apple on an
/// unsupported system keeps the whisper fallback (the plan stands
/// whisper in), and auto keeps a downloaded whisper hot behind Apple.
fn download_auto_load_plan(
	state: &AppState,
	spec: &crate::models::ModelSpec,
	apple_available: bool,
) -> Option<u64> {
	let generation = state.ai_settings_generation.load(Ordering::SeqCst);
	let settings = state.ai_settings();
	let plan = crate::plan_model_load(&settings, apple_available, |candidate| {
		state.is_model_downloaded(candidate)
	});
	let wanted = match spec.kind {
		ModelKind::Llm => plan.llm == crate::LlmPlan::Load(spec.id),
		ModelKind::Stt => plan.whisper == crate::WhisperPlan::Load(spec.id),
	};
	if wanted {
		Some(generation)
	} else {
		None
	}
}

/// Patch the settings for `spec` becoming the active engine of its
/// kind; applied to the LATEST state through mutate_ai_settings, so a
/// concurrent settings save can never be overwritten by a stale full
/// snapshot. Activating a local LLM also switches chats off an external
/// endpoint: otherwise the model would load but never be used.
fn apply_activation(settings: &mut crate::models::AiSettings, spec: &crate::models::ModelSpec) {
	match spec.kind {
		ModelKind::Llm => {
			settings.llm_mode = crate::models::LlmMode::Local;
			settings.llm_model = spec.id.to_string();
		}
		ModelKind::Stt => {
			settings.stt_model = spec.id.to_string();
		}
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
				// the settings row is only updated once the engine actually
				// loaded, so the recorded active model can never disagree
				// with the runtime; the patch applies to the LATEST state,
				// so a settings save that landed while the engine was
				// loading survives this commit
				state
					.mutate_ai_settings(|s| {
						apply_activation(s, &spec);
						Ok(())
					})
					.map(|_| ())
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

	fn temp_state(name: &str) -> (AppState, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::create_dir_all(dir.path().join("models")).expect("make models dir");
		let db = crate::db::Db::open(&dir.path().join(format!("{name}.db"))).expect("open test db");
		(AppState::new(db, dir.path().to_path_buf()), dir)
	}

	#[test]
	fn delete_is_blocked_by_download_bookkeeping_only() {
		let (state, _dir) = temp_state("blocked");
		let spec = crate::models::find_model("whisper-tiny-en", crate::models::ModelKind::Stt)
			.expect("catalog model");
		assert_eq!(super::delete_blocked(&state, spec), None);

		register_download(&state, spec.id).expect("register");
		assert!(super::delete_blocked(&state, spec).is_some(), "downloading");
		unregister_download(&state, spec.id);

		// A load of this kind is not a guard arm: the delete holds the
		// engine slot claim while re-checking this guard, and the claim
		// itself owns the loading flag - checking it here would refuse
		// every claimed delete. Load exclusivity is the claim's job.
		state.stt_loading.store(true, Ordering::SeqCst);
		assert_eq!(
			super::delete_blocked(&state, spec),
			None,
			"load exclusivity is the slot claim's job, not this guard's"
		);
		state.stt_loading.store(false, Ordering::SeqCst);
		state.llm_loading.store(true, Ordering::SeqCst);
		assert_eq!(super::delete_blocked(&state, spec), None);
	}

	#[test]
	fn a_finished_download_stays_undeletable_until_its_auto_load_starts() {
		let (state, _dir) = temp_state("handoff");
		let spec = crate::models::find_model("whisper-tiny-en", crate::models::ModelKind::Stt)
			.expect("catalog model");
		register_download(&state, spec.id).expect("register");
		// the transfer is done: the UI must stop showing "downloading"...
		super::finish_download_progress(&state, spec.id);
		assert!(!state
			.download_progress
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.contains_key(spec.id));
		// ...but until the auto-load has claimed the engine slot, a delete
		// would remove the file the load is about to open
		assert!(
			super::delete_blocked(&state, spec).is_some(),
			"delete slipped in between the download and its auto-load"
		);
		assert!(
			register_download(&state, spec.id).is_err(),
			"a second download must not reuse the slot either"
		);
		// the guard's cleanup (after the auto-load) frees everything
		unregister_download(&state, spec.id);
		assert_eq!(super::delete_blocked(&state, spec), None);
	}

	#[test]
	fn activating_a_model_records_it_for_its_kind_only() {
		use crate::models::{find_model, LlmMode, ModelKind};
		let (state, _dir) = temp_state("activate");
		let mut before = state.ai_settings();
		before.llm_mode = LlmMode::External;
		before.stt_model = "whisper-tiny-en".into();

		let llm = find_model("gemma-4-E4B", ModelKind::Llm).unwrap();
		let mut after = before.clone();
		super::apply_activation(&mut after, llm);
		assert_eq!(after.llm_model, "gemma-4-E4B");
		assert_eq!(
			after.llm_mode,
			LlmMode::Local,
			"a local model activated while chats use an endpoint must take over"
		);
		assert_eq!(after.stt_model, "whisper-tiny-en", "STT untouched");

		let stt = find_model("whisper-small-en", ModelKind::Stt).unwrap();
		let mut after = before.clone();
		super::apply_activation(&mut after, stt);
		assert_eq!(after.stt_model, "whisper-small-en");
		assert_eq!(after.llm_mode, LlmMode::External, "LLM mode untouched");
		assert_eq!(after.llm_model, before.llm_model);
	}

	#[test]
	fn activation_and_a_concurrent_settings_save_both_survive() {
		use crate::models::{find_model, LlmMode, ModelKind};
		let (state, _dir) = temp_state("activate-race");
		let state = std::sync::Arc::new(state);
		let llm = find_model("gemma-4-E4B", ModelKind::Llm).unwrap();

		// A settings save and an activation commit (which finishes a
		// multi-second engine load before touching settings) race; both
		// are poised on the barrier. Whichever lands last must not roll
		// the other back.
		let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
		let a_state = state.clone();
		let a_barrier = barrier.clone();
		let a = std::thread::spawn(move || {
			a_barrier.wait();
			a_state
				.mutate_ai_settings(|s| {
					super::apply_activation(s, llm);
					Ok(())
				})
				.expect("activation patch commits");
		});
		let b_state = state.clone();
		let b = std::thread::spawn(move || {
			barrier.wait();
			b_state
				.mutate_ai_settings(|s| {
					s.apply_updates(&serde_json::json!({ "sttLanguage": "fr-FR" }))
				})
				.expect("settings save commits");
		});
		a.join().unwrap();
		b.join().unwrap();

		assert_eq!(state.ai_settings().stt_language, "fr-FR");
		assert_eq!(state.ai_settings().llm_model, "gemma-4-E4B");
		assert_eq!(state.ai_settings().llm_mode, LlmMode::Local);
	}

	#[test]
	fn a_finished_stt_download_auto_loads_only_when_the_plan_loads_whisper() {
		use crate::models::{SpeechEngine, SttMode};
		let (state, _dir) = temp_state("auto-stt");
		let spec = crate::models::find_model("whisper-small-en", crate::models::ModelKind::Stt)
			.expect("catalog model");
		std::fs::write(state.model_path(spec), b"stub").expect("downloaded file");

		// local whisper routing: the plan loads it
		state
			.mutate_ai_settings(|s| {
				s.stt_engine = SpeechEngine::Whisper;
				s.stt_mode = SttMode::Local;
				s.stt_model = spec.id.to_string();
				Ok(())
			})
			.expect("commit");
		assert!(
			super::download_auto_load_plan(&state, spec, true).is_some(),
			"whisper routing loads the downloaded whisper model"
		);

		// external STT selected: no whisper load even though it is the
		// recorded stt model
		state
			.mutate_ai_settings(|s| {
				s.stt_mode = SttMode::External;
				s.ext_stt_base_url = "http://localhost:9000".into();
				Ok(())
			})
			.expect("commit");
		assert!(
			super::download_auto_load_plan(&state, spec, true).is_none(),
			"external STT routing must not load whisper"
		);

		// explicit Apple speech on a supporting system: whisper freed
		state
			.mutate_ai_settings(|s| {
				s.stt_mode = SttMode::Local;
				s.stt_engine = SpeechEngine::Apple;
				s.ext_stt_base_url = String::new();
				Ok(())
			})
			.expect("commit");
		assert!(
			super::download_auto_load_plan(&state, spec, true).is_none(),
			"explicit Apple speech must not load whisper"
		);

		// explicit Apple on an unsupported system: whisper stands in
		assert!(
			super::download_auto_load_plan(&state, spec, false).is_some(),
			"the Apple-unsupported fallback loads whisper"
		);

		// auto keeps a downloaded whisper hot behind Apple
		state
			.mutate_ai_settings(|s| {
				s.stt_engine = SpeechEngine::Auto;
				Ok(())
			})
			.expect("commit");
		assert!(
			super::download_auto_load_plan(&state, spec, true).is_some(),
			"auto keeps whisper loaded as the fallback"
		);

		// a download of a model the plan would not pick loads nothing
		let other = crate::models::find_model("whisper-tiny-en", crate::models::ModelKind::Stt)
			.expect("catalog model");
		std::fs::write(state.model_path(other), b"stub").expect("downloaded file");
		assert!(
			super::download_auto_load_plan(&state, other, true).is_none(),
			"only the plan's model auto-loads"
		);
	}

	#[test]
	fn a_finished_llm_download_respects_the_current_llm_mode() {
		use crate::models::LlmMode;
		let (state, _dir) = temp_state("auto-llm");
		let spec = crate::models::find_model("gemma-4-E4B", crate::models::ModelKind::Llm)
			.expect("catalog model");
		std::fs::write(state.model_path(spec), b"stub").expect("downloaded file");

		state
			.mutate_ai_settings(|s| {
				s.llm_mode = LlmMode::Local;
				s.llm_model = spec.id.to_string();
				Ok(())
			})
			.expect("commit");
		assert!(
			super::download_auto_load_plan(&state, spec, true).is_some(),
			"local mode loads the downloaded llm"
		);

		state
			.mutate_ai_settings(|s| {
				s.llm_mode = LlmMode::External;
				s.ext_llm_base_url = "http://localhost:9001".into();
				Ok(())
			})
			.expect("commit");
		assert!(
			super::download_auto_load_plan(&state, spec, true).is_none(),
			"external llm routing must not load the local engine"
		);
	}

	/// A download that ends (or never really starts) must release its slot,
	/// or the model is stuck as "downloading" until app restart.
	#[test]
	fn registration_is_atomic_and_cleanup_releases_the_slot() {
		let (state, _dir) = temp_state("slot");
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

	struct RecordingEvents(std::sync::Mutex<Vec<String>>);

	impl crate::StatusEvents for RecordingEvents {
		fn llm_status(&self, status: &crate::models::EngineStatus) {
			self.0
				.lock()
				.unwrap()
				.push(format!("llm:{:?}:{:?}", status.state, status.model_id));
		}
		fn stt_status(&self, status: &crate::models::EngineStatus) {
			self.0
				.lock()
				.unwrap()
				.push(format!("stt:{:?}:{:?}", status.state, status.model_id));
		}
	}

	/// The production delete wiring through the real runtime slots: no
	/// engine is resident in a test state, so nothing is published and
	/// the app copy still goes. The cache list is injected so the test
	/// never touches the developer's real hub cache.
	#[test]
	fn delete_model_events_removes_the_app_copy_through_the_real_slot() {
		use std::time::Duration;
		let (state, _dir) = temp_state("del-cmd");
		let spec = crate::models::find_model("whisper-small-en", crate::models::ModelKind::Stt)
			.expect("catalog model");
		std::fs::write(state.model_path(spec), b"stub").expect("model file");
		let events = RecordingEvents(std::sync::Mutex::new(Vec::new()));

		super::delete_model_events(&state, &events, spec, &[], Duration::from_secs(5))
			.expect("the delete succeeds");

		assert!(
			!state.model_path(spec).is_file(),
			"the app copy is removed through the real slot path"
		);
		assert!(
			events.0.lock().unwrap().is_empty(),
			"no engine was resident: nothing to publish: {:?}",
			events.0.lock().unwrap()
		);

		let err = super::delete_model_events(&state, &events, spec, &[], Duration::from_secs(5))
			.expect_err("nothing is left to delete");
		assert_eq!(err, "model file not found");
	}
}
