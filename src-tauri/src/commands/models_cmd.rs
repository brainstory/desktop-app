use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;
use tauri::ipc::Channel;
use tauri::{Emitter, Manager, State};

use crate::models::{
	download_model_file_with_pause, find_model, kv_bytes_per_token, model_url, part_path,
	ModelKind, PauseEntry,
};
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
		// Snapshot the progress and paused markers under the locks, then
		// release them: the loop below stats every catalog file (and can
		// scan hub-cache dirs), and holding the mutexes across that would
		// block every download event that arrives mid-scan.
		let progress: std::collections::HashMap<String, f64> = state
			.download_progress
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.iter()
			.map(|(id, pct)| (id.clone(), *pct))
			.collect();
		let paused: std::collections::HashMap<String, f64> = state
			.download_pauses
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.iter()
			.filter(|(_, entry)| entry.paused)
			.map(|(id, entry)| (id.clone(), entry.pct))
			.collect();
		let build = |kind: ModelKind| -> Vec<serde_json::Value> {
			let list: &[crate::models::ModelSpec] = match kind {
				ModelKind::Llm => &crate::models::LLM_MODELS,
				ModelKind::Stt => &crate::models::STT_MODELS,
			};
			list.iter()
				.map(|spec| {
					// an actively downloading model's live pct wins over
					// the marker a pause left behind
					let active_pct = progress.get(spec.id).copied();
					let paused_pct = paused.get(spec.id).copied();
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
						progress: active_pct.or(paused_pct),
						paused: active_pct.is_none() && paused_pct.is_some(),
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

/// Bytes-per-second and ETA from consecutive progress reports. Progress
/// arrives every ~2 MB (or on completion), so each delta is a meaningful
/// sample; the rate is lightly smoothed so the label does not flicker,
/// and the ETA derives from the smoothed rate.
struct ProgressRate {
	total: u64,
	last: Option<(std::time::Instant, u64)>,
	smoothed: Option<f64>,
}

impl ProgressRate {
	fn new(total: u64) -> Self {
		Self {
			total,
			last: None,
			smoothed: None,
		}
	}

	/// Feed a progress percentage; None when there is nothing new to say
	/// (unknown size, the first report, or a non-advancing sample).
	fn update(&mut self, pct: f64) -> Option<(f64, u64)> {
		if self.total == 0 || pct < 0.0 {
			return None;
		}
		let bytes = ((pct / 100.0) * self.total as f64).round() as u64;
		self.update_at(bytes, std::time::Instant::now())
	}

	/// [`Self::update`] with an injected clock, so the arithmetic is
	/// testable without sleeping.
	fn update_at(&mut self, bytes: u64, now: std::time::Instant) -> Option<(f64, u64)> {
		let (previous_at, previous_bytes) = match self.last {
			// the first report is the baseline, nothing to measure yet
			None => {
				self.last = Some((now, bytes));
				return None;
			}
			Some(previous) => previous,
		};
		let elapsed = now.duration_since(previous_at).as_secs_f64();
		let advanced = bytes.saturating_sub(previous_bytes) as f64;
		// a non-advancing sample keeps the old baseline, so the next
		// advancing report still spans the original interval
		if elapsed <= 0.0 || advanced <= 0.0 {
			return None;
		}
		self.last = Some((now, bytes));
		let sample = advanced / elapsed;
		let rate = match self.smoothed {
			Some(previous_rate) => previous_rate * 0.7 + sample * 0.3,
			None => sample,
		};
		self.smoothed = Some(rate);
		let remaining = self.total.saturating_sub(bytes) as f64;
		let eta = (remaining / rate).ceil();
		if !eta.is_finite() || eta < 0.0 || eta > u64::MAX as f64 {
			return None;
		}
		Some((rate, eta as u64))
	}
}

impl Drop for DownloadGuard {
	fn drop(&mut self) {
		if let Some(state) = self.app.try_state::<AppState>() {
			unregister_download(&state, &self.model_id);
		}
	}
}

/// Insert a download into the bookkeeping maps under one critical
/// section, so a concurrent `cancel_download`/`pause_download` or
/// `download_model` can never observe a half-registered download. Err
/// means this model is already downloading. Returns the fresh cancel and
/// pause tokens.
fn register_download(
	state: &AppState,
	model_id: &str,
) -> Result<(Arc<AtomicBool>, Arc<AtomicBool>), String> {
	// Lock ordering (progress, then cancels, then pauses) must match
	// unregister_download.
	let mut progress = state
		.download_progress
		.lock()
		.unwrap_or_else(|e| e.into_inner());
	let mut cancels = state
		.download_cancels
		.lock()
		.unwrap_or_else(|e| e.into_inner());
	let mut pauses = state
		.download_pauses
		.lock()
		.unwrap_or_else(|e| e.into_inner());
	// a cancel entry without progress is a finished download whose
	// auto-load still holds the slot
	if progress.contains_key(model_id) || cancels.contains_key(model_id) {
		return Err("model is already downloading".into());
	}
	let cancel = Arc::new(AtomicBool::new(false));
	let pause = Arc::new(AtomicBool::new(false));
	progress.insert(model_id.to_string(), 0.0);
	cancels.insert(model_id.to_string(), cancel.clone());
	// a fresh attempt supersedes a paused one: Resume restarts the
	// transfer from the kept `.part` under a new control
	pauses.insert(
		model_id.to_string(),
		PauseEntry {
			token: pause.clone(),
			pct: 0.0,
			paused: false,
		},
	);
	Ok((cancel, pause))
}

/// The transfer finished: stop reporting the model as downloading, but
/// keep its cancel entry (which still holds the slot) until the
/// DownloadGuard drops after the auto-load. A paused marker cannot
/// survive a completed download either.
fn finish_download_progress(state: &AppState, model_id: &str) {
	state
		.download_progress
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.remove(model_id);
	state
		.download_pauses
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.remove(model_id);
}

/// Remove a download's bookkeeping entries (inverse of register_download).
/// A *paused* entry is kept on purpose: Resume needs its recorded
/// progress, and it is cleared by the next registration, a cancel, a
/// finished download or a model delete.
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
	let mut pauses = state
		.download_pauses
		.lock()
		.unwrap_or_else(|e| e.into_inner());
	let paused = pauses
		.get(model_id)
		.map(|entry| entry.paused)
		.unwrap_or(false);
	if !paused {
		pauses.remove(model_id);
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
	// Every download event carries the label, so UI surfaces outside the
	// settings page (the sidebar progress pill, the chat setup notice)
	// can name the model without fetching the catalog.
	let spec_label = spec.label;

	// Already present (app copy or hub cache)? Nothing to download -
	// the UI hides the button, this covers races and stale cards. A
	// hub-cache file whose size disagrees with the pinned spec is NOT
	// "already downloaded": a truncated or foreign copy must be
	// re-downloadable.
	let existing = state.resolve_model_file(&spec);
	let app_copy = state.model_path(&spec);
	let size_matches = existing.as_ref().is_some_and(|path| {
		std::fs::metadata(path)
			.map(|m| m.len() == spec.size_bytes)
			.unwrap_or(false)
	});
	if existing.is_some() && (existing == Some(app_copy) || size_matches) {
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

	// Pre-flight free space: a staged `.part` that can grow past the
	// volume's limit fails late (ENOSPC mid-transfer). Refuse up front
	// when even the model plus a 64 MiB scratch margin does not fit,
	// crediting whatever the leftover `.part` already holds.
	{
		let existing_part_len = std::fs::metadata(part_path(&dest))
			.map(|meta| meta.len())
			.unwrap_or(0);
		let needed = spec
			.size_bytes
			.saturating_sub(existing_part_len)
			.saturating_add(64 * 1024 * 1024);
		let available = fs4::available_space(&cache).map_err(|e| e.to_string())?;
		if available < needed {
			let short_by_mb = (needed - available) / (1024 * 1024);
			return Err(format!(
				"not enough disk space: this model needs about {} MB more - free some room and try again",
				short_by_mb.max(1)
			));
		}
	}

	// This download gets its own cancel and pause tokens, fully
	// independent of the generation token - chatting must never kill a
	// download and vice versa.
	let (cancel, pause) = register_download(&state, &model_id)?;
	// The guard exists before the spawn, so nothing between registration
	// and the spawned task can leak the bookkeeping entries.
	let guard = DownloadGuard {
		app: app.clone(),
		model_id: model_id.clone(),
	};

	let app_handle = app.clone();
	let hf_endpoint = crate::models::hf_endpoint(&state.ai_settings());
	// re-vet the resolved endpoint (setting, mirror or HF_ENDPOINT env)
	// before a byte moves or the token is attached
	crate::models::validate_endpoint_url(&hf_endpoint).map_err(|e| {
		format!(
			"invalid HuggingFace download endpoint (Settings, AI Models, mirror - or HF_ENDPOINT): {e}"
		)
	})?;
	let url = model_url(&spec, &hf_endpoint);
	let hf_token = state.ai_settings().hf_token;

	tauri::async_runtime::spawn(async move {
		let _guard = guard;
		{
			let state = app_handle.state::<AppState>();
			let model_id = model_id.clone();
			// Speed/ETA come from consecutive progress reports: bytes
			// derive from pct × the catalog size, the delta between
			// reports gives the rate (lightly smoothed so the label does
			// not flicker).
			let mut rate = ProgressRate::new(spec.size_bytes);
			let mut on_progress = |pct: f64| {
				state
					.download_progress
					.lock()
					.unwrap_or_else(|e| e.into_inner())
					.insert(model_id.clone(), pct);
				let _ = on_event.send(json!({ "kind": "progress", "pct": pct }));
				let mut payload = json!({
					"modelId": model_id,
					"label": spec_label,
					"kind": "progress",
					"pct": pct
				});
				if let Some((bytes_per_sec, eta_secs)) = rate.update(pct) {
					payload["bytesPerSec"] = json!(bytes_per_sec);
					payload["etaSecs"] = json!(eta_secs);
				}
				let _ = app_handle.emit("model-download", payload);
			};
			let result = download_model_file_with_pause(
				&url,
				&dest,
				spec.size_bytes,
				spec.sha256,
				&hf_token,
				&cancel,
				&pause,
				&mut on_progress,
			)
			.await;

			match result {
				Ok(()) => {
					// publish the blob under snapshots/<sha>/<file> so
					// discovery (ours and other HF tools) sees it. A publish
					// failure means the model is NOT resolvable
					// (resolve_model_file reads snapshots), so it must be
					// reported as a failure instead of "done": the UI would
					// otherwise show a downloaded model the app cannot load.
					if let Err(e) = crate::models::materialize_snapshot(&cache, &spec) {
						log::error!("could not create the cache snapshot for {}: {e}", spec.id);
						let message = format!(
							"the model file was downloaded but could not be published to the cache: {e}"
						);
						let _ =
							on_event.send(json!({ "kind": "error", "message": message.clone() }));
						let _ = app_handle.emit(
							"model-download",
							json!({
								"modelId": model_id,
								"label": spec_label,
								"kind": "error",
								"message": message
							}),
						);
						return;
					}
					let _ = on_event.send(json!({ "kind": "done" }));
					let _ = app_handle.emit(
						"model-download",
						json!({ "modelId": model_id, "label": spec_label, "kind": "done" }),
					);
					// The download itself is finished: the model stops
					// reporting "downloading" while its engine spins up,
					// but the guard (and its cancel entry) lives until the
					// auto-load below is done, so delete_model can't slip
					// in and remove the file the load is about to open.
					finish_download_progress(&app_handle.state::<AppState>(), &model_id);
					// Load the model now when the loader's plan calls for
					// it - or, when nothing local is set up for this kind
					// yet, after activating it first (see DownloadPlan).
					let state = app_handle.state::<AppState>();
					let load_generation =
						match download_plan(&state, &spec, crate::apple::speech_available()) {
							DownloadPlan::LoadSelected(generation) => Some(generation),
							DownloadPlan::ActivateThenLoad => {
								match state.mutate_ai_settings(|s| {
									apply_activation(s, &spec);
									Ok(())
								}) {
									Ok((_settings, generation)) => {
										// the models list's "Active" badge must
										// move without waiting for another event
										let _ = app_handle.emit(
											"model-download",
											json!({
												"modelId": model_id,
												"label": spec_label,
												"kind": "activated"
											}),
										);
										Some(generation)
									}
									Err(e) => {
										log::error!(
											"could not activate the downloaded model {}: {e}",
											spec.id
										);
										None
									}
								}
							}
							DownloadPlan::Nothing => None,
						};
					if let Some(generation) = load_generation {
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
									"label": spec_label,
									"kind": "load-error",
									"message": e
								}),
							);
						}
					}
				}
				Err(e) => {
					// A user cancel and a user pause are not failures:
					// each gets its own kind (no red "download failed" for
					// a button the user pressed). A cancel discards the
					// staged prefix; a pause keeps it and records where the
					// transfer stopped so Resume continues from there.
					let is_cancelled = e == crate::models::DOWNLOAD_CANCELLED;
					let is_paused = e == crate::models::DOWNLOAD_PAUSED;
					let state = app_handle.state::<AppState>();
					let kind = if is_cancelled {
						"cancelled"
					} else if is_paused {
						"paused"
					} else {
						"error"
					};
					let mut payload = json!({
						"modelId": model_id,
						"label": spec_label,
						"kind": kind
					});
					if is_cancelled {
						// a cancel discards the staged prefix: no paused
						// state may linger for a retry to resume from
						state
							.download_pauses
							.lock()
							.unwrap_or_else(|e| e.into_inner())
							.remove(&model_id);
						let _ = on_event.send(json!({ "kind": "cancelled" }));
					} else if is_paused {
						// record where the transfer stopped; unregister
						// keeps paused entries, so Resume (and a fresh page
						// load) can show the progress
						let pct = state
							.download_progress
							.lock()
							.unwrap_or_else(|e| e.into_inner())
							.get(&model_id)
							.copied()
							.unwrap_or(0.0);
						let mut pauses = state
							.download_pauses
							.lock()
							.unwrap_or_else(|e| e.into_inner());
						if let Some(entry) = pauses.get_mut(&model_id) {
							entry.pct = pct;
							entry.paused = true;
						}
						drop(pauses);
						payload["pct"] = json!(pct);
						let _ = on_event.send(json!({ "kind": "paused", "pct": pct }));
					} else {
						payload["message"] = json!(e);
						let _ = on_event.send(json!({ "kind": "error", "message": e }));
					}
					let _ = app_handle.emit("model-download", payload);
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
/// whether anything was removed. `verified` carries the snapshot-file
/// hashes taken before the engine-slot claim.
fn delete_model_files(
	state: &AppState,
	spec: &crate::models::ModelSpec,
	caches: &[std::path::PathBuf],
	verified: &crate::models::VerifiedSnapshotFiles,
) -> Result<bool, String> {
	let mut deleted = false;
	let app_path = state.model_path(spec);
	// a paused or interrupted transfer's staging file goes with the model
	if crate::models::remove_staged_part(&app_path)? {
		deleted = true;
	}
	if app_path.is_file() {
		std::fs::remove_file(&app_path).map_err(|e| e.to_string())?;
		deleted = true;
	}
	for cache in caches {
		if crate::models::remove_staged_part(&crate::models::hf_blob_path(cache, spec))? {
			deleted = true;
		}
		deleted |= crate::models::remove_cached_model(cache, spec, verified)?;
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
	// Hash verification of snapshot files runs BEFORE the engine-slot
	// claim: the delete can wait a long time for an in-flight model load,
	// and hashing multi-GB files while holding that claim would extend
	// the window in which every other engine operation is blocked.
	let verified = crate::models::verified_snapshot_files(caches, spec);
	let delete_files = || delete_model_files(state, spec, caches, &verified);
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
		// a paused download's marker goes with the model (its staged file
		// is removed by the delete path itself)
		state
			.download_pauses
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.remove(&model_id);
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

/// Pause an in-flight download: the transfer stops within a tick and
/// KEEPS its staged `.part`, and the UI is told to offer Resume (a fresh
/// `download_model` continues from the prefix).
#[tauri::command]
pub fn pause_download(state: State<'_, AppState>, model_id: String) -> Result<(), String> {
	let pauses = state
		.download_pauses
		.lock()
		.unwrap_or_else(|e| e.into_inner());
	match pauses.get(&model_id) {
		Some(entry) if !entry.paused => {
			entry.token.store(true, Ordering::Relaxed);
			Ok(())
		}
		_ => Err(format!("no download in progress for {model_id}")),
	}
}

/// What to do with a finished download. Routed through the same plan the
/// model loader uses instead of comparing model ids, so the auto-load can
/// never contradict the selected routing: external STT or an explicit
/// Apple Speech selection loads no whisper even when it is the recorded
/// stt model, and external LLM mode loads no local engine. Explicit Apple
/// on an unsupported system keeps the whisper fallback (the plan stands
/// whisper in), and auto keeps a downloaded whisper hot behind Apple.
enum DownloadPlan {
	/// The model the loader's plan wants: load it now, at this settings
	/// generation.
	LoadSelected(u64),
	/// Nothing local is set up for this kind yet, so the finished
	/// download is the first usable model: make it the selected one and
	/// load it. Without this, following the "recommended model" hint
	/// (smallest, not the catalog default) ended with a downloaded model
	/// that was never used until the user found the "Use" button.
	ActivateThenLoad,
	/// Another model already handles this kind: leave the selection
	/// alone (the new model stays available behind "Use").
	Nothing,
}

fn download_plan(
	state: &AppState,
	spec: &crate::models::ModelSpec,
	apple_available: bool,
) -> DownloadPlan {
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
		return DownloadPlan::LoadSelected(generation);
	}
	// Missing is only ever reached with local routing (externals plan
	// External/Unload), so no extra external check is needed here.
	let nothing_local = match spec.kind {
		ModelKind::Llm => plan.llm == crate::LlmPlan::Missing,
		ModelKind::Stt => plan.whisper == crate::WhisperPlan::Missing,
	};
	if nothing_local {
		DownloadPlan::ActivateThenLoad
	} else {
		DownloadPlan::Nothing
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
		use super::DownloadPlan;
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
			matches!(
				super::download_plan(&state, spec, true),
				DownloadPlan::LoadSelected(_)
			),
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
			matches!(
				super::download_plan(&state, spec, true),
				DownloadPlan::Nothing
			),
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
			matches!(
				super::download_plan(&state, spec, true),
				DownloadPlan::Nothing
			),
			"explicit Apple speech must not load whisper"
		);

		// explicit Apple on an unsupported system: whisper stands in
		assert!(
			matches!(
				super::download_plan(&state, spec, false),
				DownloadPlan::LoadSelected(_)
			),
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
			matches!(
				super::download_plan(&state, spec, true),
				DownloadPlan::LoadSelected(_)
			),
			"auto keeps whisper loaded as the fallback"
		);

		// a second downloaded whisper while one is already selected and
		// downloaded: the selection stays put
		let other = crate::models::find_model("whisper-tiny-en", crate::models::ModelKind::Stt)
			.expect("catalog model");
		std::fs::write(state.model_path(other), b"stub").expect("downloaded file");
		assert!(
			matches!(
				super::download_plan(&state, other, true),
				DownloadPlan::Nothing
			),
			"only the plan's model auto-loads"
		);
	}

	#[test]
	fn a_finished_llm_download_respects_the_current_llm_mode() {
		use super::DownloadPlan;
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
			matches!(
				super::download_plan(&state, spec, true),
				DownloadPlan::LoadSelected(_)
			),
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
			matches!(
				super::download_plan(&state, spec, true),
				DownloadPlan::Nothing
			),
			"external llm routing must not load the local engine"
		);
	}

	/// The first-run trap this plan fixes: the setup card recommends the
	/// smallest model, but only the catalog default was ever auto-loaded,
	/// so following the recommendation left a downloaded model unused.
	#[test]
	fn a_download_activates_when_nothing_local_is_set_up() {
		use super::DownloadPlan;
		use crate::models::{LlmMode, SpeechEngine, SttMode};
		let (state, _dir) = temp_state("auto-first");

		// fresh install, local routing, the selected default not downloaded
		state
			.mutate_ai_settings(|s| {
				s.llm_mode = LlmMode::Local;
				s.llm_model = "gemma-4-E2B-qat".into();
				s.stt_engine = SpeechEngine::Whisper;
				s.stt_mode = SttMode::Local;
				s.stt_model = "whisper-base-en".into();
				Ok(())
			})
			.expect("commit");

		let llm = crate::models::find_model("minicpm5-2b", crate::models::ModelKind::Llm)
			.expect("catalog model");
		assert!(
			matches!(
				super::download_plan(&state, llm, true),
				DownloadPlan::ActivateThenLoad
			),
			"the first downloaded llm must become the active one"
		);

		let stt = crate::models::find_model("whisper-tiny-en", crate::models::ModelKind::Stt)
			.expect("catalog model");
		assert!(
			matches!(
				super::download_plan(&state, stt, true),
				DownloadPlan::ActivateThenLoad
			),
			"the first downloaded whisper model must become the active one"
		);

		// auto engine with Apple available also wants a fallback whisper:
		// nothing is downloaded yet, so the first whisper still activates
		state
			.mutate_ai_settings(|s| {
				s.stt_engine = SpeechEngine::Auto;
				Ok(())
			})
			.expect("commit");
		assert!(
			matches!(
				super::download_plan(&state, stt, true),
				DownloadPlan::ActivateThenLoad
			),
			"Apple-auto still activates the first downloaded whisper fallback"
		);

		// an external LLM is not "nothing set up": a local download must
		// never steal the selection from the endpoint
		state
			.mutate_ai_settings(|s| {
				s.llm_mode = LlmMode::External;
				s.ext_llm_base_url = "http://localhost:9001".into();
				Ok(())
			})
			.expect("commit");
		assert!(
			matches!(
				super::download_plan(&state, llm, true),
				DownloadPlan::Nothing
			),
			"external LLM routing keeps the local download passive"
		);

		// once a local llm is usable, later downloads stay passive
		state
			.mutate_ai_settings(|s| {
				s.llm_mode = LlmMode::Local;
				s.llm_model = llm.id.to_string();
				Ok(())
			})
			.expect("commit");
		std::fs::write(state.model_path(llm), b"stub").expect("downloaded file");
		let gemma = crate::models::find_model("gemma-4-E2B-qat", crate::models::ModelKind::Llm)
			.expect("catalog model");
		assert!(
			matches!(
				super::download_plan(&state, gemma, true),
				DownloadPlan::Nothing
			),
			"a usable selection must survive another download"
		);
	}

	/// A download that ends (or never really starts) must release its slot,
	/// or the model is stuck as "downloading" until app restart.
	#[test]
	fn registration_is_atomic_and_cleanup_releases_the_slot() {
		let (state, _dir) = temp_state("slot");
		let (cancel, _pause) = register_download(&state, "m").expect("first registration wins");
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

	#[test]
	fn pause_bookkeeping_survives_the_task_and_a_resume_resets_it() {
		let (state, _dir) = temp_state("pause");
		let (cancel, pause) = register_download(&state, "m").expect("register");
		assert!(!cancel.load(Ordering::Relaxed));
		assert!(!pause.load(Ordering::Relaxed));

		// the UI pauses the transfer...
		pause.store(true, Ordering::Relaxed);
		// ...and the download task records where it stopped before its
		// guard drops
		{
			let mut pauses = state
				.download_pauses
				.lock()
				.unwrap_or_else(|e| e.into_inner());
			let entry = pauses.get_mut("m").expect("pause entry");
			entry.pct = 42.0;
			entry.paused = true;
		}
		unregister_download(&state, "m");

		// the paused entry outlives its task: Resume needs its pct
		let paused = state
			.download_pauses
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.get("m")
			.map(|entry| (entry.paused, entry.pct));
		assert_eq!(paused, Some((true, 42.0)));

		// a resume (a fresh registration) clears the marker
		let _ = register_download(&state, "m").expect("resume registers");
		let resumed = state
			.download_pauses
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.get("m")
			.map(|entry| (entry.paused, entry.pct));
		assert_eq!(resumed, Some((false, 0.0)));

		// a plain (never paused) unregister cleans up everything
		unregister_download(&state, "m");
		assert!(!state
			.download_pauses
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.contains_key("m"));
	}

	#[test]
	fn a_completed_download_clears_a_stale_pause_marker() {
		let (state, _dir) = temp_state("pausefin");
		let _ = register_download(&state, "m").expect("register");
		{
			let mut pauses = state
				.download_pauses
				.lock()
				.unwrap_or_else(|e| e.into_inner());
			if let Some(entry) = pauses.get_mut("m") {
				entry.pct = 90.0;
				entry.paused = true;
			}
		}
		super::finish_download_progress(&state, "m");
		assert!(!state
			.download_pauses
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.contains_key("m"));
		unregister_download(&state, "m");
	}

	#[test]
	fn progress_rate_reports_smoothed_speed_and_eta() {
		let mut rate = super::ProgressRate::new(1_000_000);
		let start = std::time::Instant::now();
		// the first report has no baseline
		assert_eq!(rate.update_at(0, start), None);
		// 100 kB in 2 s -> 50 kB/s, 900 kB left -> 18 s
		let first = rate.update_at(100_000, start + std::time::Duration::from_secs(2));
		assert_eq!(first, Some((50_000.0, 18)));
		// a non-advancing sample is ignored
		assert_eq!(
			rate.update_at(100_000, start + std::time::Duration::from_secs(4)),
			None
		);
		// 200 kB more in 2 s (100 kB/s sample): 0.7*50k + 0.3*100k = 65 kB/s
		let second = rate.update_at(300_000, start + std::time::Duration::from_secs(4));
		assert_eq!(second, Some((65_000.0, 11)));
		// an unknown total or an indeterminate pct reports nothing
		let mut unknown = super::ProgressRate::new(0);
		assert_eq!(unknown.update(10.0), None);
		let mut known = super::ProgressRate::new(1_000);
		assert_eq!(known.update(-1.0), None);
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
