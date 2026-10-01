//! Runtime state: engines, statuses, load/swap/rollback, download
//! bookkeeping.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use super::ai_settings::AiSettings;
use super::catalog::{find_model, ModelSpec};
use super::download::{hf_cache_model_path, hf_hub_cache_candidates};
use crate::db::Db;
use crate::llm::LocalLlm;
use crate::stt::SttEngine;

pub struct Runtime {
	pub backend: Option<Arc<llama_cpp_2::llama_backend::LlamaBackend>>,
	pub llm: Option<Arc<LocalLlm>>,
	pub stt: Option<Arc<SttEngine>>,
}

/// Lifecycle of one engine slot. Serialized lowercase so the existing
/// frontend contract (`state === "ready"` etc.) is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineState {
	Ready,
	Loading,
	Error,
	Missing,
	External,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineStatus {
	pub state: EngineState,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
}

impl EngineStatus {
	pub fn new(state: EngineState, model_id: Option<&str>, error: Option<&str>) -> Self {
		Self {
			state,
			model_id: model_id.map(|s| s.into()),
			error: error.map(|s| s.into()),
		}
	}

	pub fn ready(model_id: Option<&str>) -> Self {
		Self::new(EngineState::Ready, model_id, None)
	}

	pub fn loading(model_id: &str) -> Self {
		Self::new(EngineState::Loading, Some(model_id), None)
	}

	pub fn error(model_id: Option<&str>, error: &str) -> Self {
		Self::new(EngineState::Error, model_id, Some(error))
	}

	pub fn missing() -> Self {
		Self::new(EngineState::Missing, None, None)
	}

	pub fn external() -> Self {
		Self::new(EngineState::External, None, None)
	}
}

pub struct AppState {
	pub db: Db,
	pub data_dir: PathBuf,
	/// Read-through cache of the AI settings (three keychain reads plus a
	/// dozen DB rows on every load); invalidated by save_ai_settings.
	pub ai_settings_cache: std::sync::RwLock<Option<AiSettings>>,
	pub runtime: std::sync::Mutex<Runtime>,
	pub llm_status: std::sync::Mutex<EngineStatus>,
	pub stt_status: std::sync::Mutex<EngineStatus>,
	/// Cancel token for the in-flight LLM generation. Only generations use
	/// this - downloads get their own token in `download_cancels`.
	pub generation_cancel: std::sync::Mutex<Arc<AtomicBool>>,
	/// model id -> progress percentage for in-flight downloads
	pub download_progress: std::sync::Mutex<std::collections::HashMap<String, f64>>,
	/// model id -> cancel token for in-flight downloads
	pub download_cancels: std::sync::Mutex<std::collections::HashMap<String, Arc<AtomicBool>>>,
	/// guards so only one load per engine kind runs at a time
	pub llm_loading: AtomicBool,
	pub stt_loading: AtomicBool,
	/// when the tray icon is hidden, closing the window quits the app
	/// (otherwise it would keep running with no way to reach it)
	pub quit_on_close: AtomicBool,
}

/// Whisper reads the model file fresh on each load: after a failed
/// reload of the active model (its engine already dropped), retrying the
/// same model recovers from a transient failure (339b19e).
const STT_ROLLBACK_ON_SAME: bool = true;
/// An identical llama reload fails deterministically on the same mmap;
/// retrying the model that just failed is pointless.
const LLM_ROLLBACK_ON_SAME: bool = false;

/// The model to restore after `failed` did not load: the previously
/// loaded one - unless that is `failed` itself and the engine kind does
/// not retry an identical load.
fn rollback_candidate(
	prev: Option<ModelSpec>,
	failed: &ModelSpec,
	rollback_on_same: bool,
) -> Option<ModelSpec> {
	prev.filter(|prev| rollback_on_same || prev.id != failed.id)
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
	// A poisoned lock still holds usable state; recover instead of
	// panicking on every later call.
	mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl AppState {
	pub fn new(db: Db, data_dir: PathBuf) -> Self {
		Self {
			db,
			data_dir,
			ai_settings_cache: std::sync::RwLock::new(None),
			runtime: std::sync::Mutex::new(Runtime {
				backend: None,
				llm: None,
				stt: None,
			}),
			llm_status: std::sync::Mutex::new(EngineStatus::new(EngineState::Missing, None, None)),
			stt_status: std::sync::Mutex::new(EngineStatus::new(EngineState::Missing, None, None)),
			generation_cancel: std::sync::Mutex::new(Arc::new(AtomicBool::new(false))),
			download_progress: std::sync::Mutex::new(std::collections::HashMap::new()),
			download_cancels: std::sync::Mutex::new(std::collections::HashMap::new()),
			llm_loading: AtomicBool::new(false),
			stt_loading: AtomicBool::new(false),
			quit_on_close: AtomicBool::new(false),
		}
	}

	/// The AI settings, from the read-through cache when warm. AiSettings
	/// is only written through [`Self::save_ai_settings`], so the cache
	/// can never go stale.
	pub fn ai_settings(&self) -> AiSettings {
		let cache = self
			.ai_settings_cache
			.read()
			.unwrap_or_else(|e| e.into_inner());
		if let Some(cached) = cache.as_ref() {
			return cached.clone();
		}
		drop(cache);
		let loaded = AiSettings::load(&self.db);
		*self
			.ai_settings_cache
			.write()
			.unwrap_or_else(|e| e.into_inner()) = Some(loaded.clone());
		loaded
	}

	/// Persist settings and refresh the cache in one step, so a failed
	/// write never leaves a cache disagreeing with the database.
	pub fn save_ai_settings(&self, settings: &AiSettings) -> Result<(), String> {
		// the cache mirrors what is stored, so secrets compare against it
		// instead of a fresh keychain read per save
		let previous = self.ai_settings();
		settings.save(&self.db, &previous)?;
		*self
			.ai_settings_cache
			.write()
			.unwrap_or_else(|e| e.into_inner()) = Some(settings.clone());
		Ok(())
	}

	pub fn models_dir(&self) -> PathBuf {
		self.data_dir.join("models")
	}

	pub fn model_path(&self, spec: &ModelSpec) -> PathBuf {
		self.models_dir().join(spec.filename)
	}

	/// Where this model's file can be loaded from, if anywhere: the
	/// app-managed copy first, then a file someone else already
	/// downloaded into the HuggingFace hub cache (hf CLI, other tools).
	pub fn resolve_model_file(&self, spec: &ModelSpec) -> Option<PathBuf> {
		let app_copy = self.model_path(spec);
		if app_copy.is_file() {
			return Some(app_copy);
		}
		hf_hub_cache_candidates()
			.iter()
			.find_map(|cache| hf_cache_model_path(cache, spec))
	}

	pub fn is_model_downloaded(&self, spec: &ModelSpec) -> bool {
		self.resolve_model_file(spec).is_some()
	}

	pub fn emit_llm_status(&self, app: &AppHandle) {
		let status = lock(&self.llm_status).clone();
		if let Err(e) = app.emit("llm-status", status) {
			log::warn!("failed to emit llm-status: {e}");
		}
	}

	pub fn emit_stt_status(&self, app: &AppHandle) {
		let status = lock(&self.stt_status).clone();
		if let Err(e) = app.emit("stt-status", status) {
			log::warn!("failed to emit stt-status: {e}");
		}
	}

	/// Load the given LLM model file into the runtime. Blocking; call from a
	/// background thread.
	///
	/// The previously loaded engine is dropped before the new file is
	/// mmap'd (peak memory stays at one model). If the new file fails to
	/// load or was deleted mid-load, the previous model is loaded back from
	/// disk, so a failed switch never leaves the app without a working LLM.
	///
	/// Err means the switch did NOT happen (a busy load, or the new model
	/// failed and could not be rolled back to); callers must not persist
	/// the new model as active on Err.
	pub fn load_llm(&self, app: &AppHandle, spec: &ModelSpec) -> Result<(), String> {
		// One load at a time: a second activate while the first is running
		// would mmap two multi-GB models simultaneously. Refuse instead of
		// silently ignoring, so callers can't persist a divergent active
		// model while a different load is in flight.
		if self.llm_loading.swap(true, Ordering::SeqCst) {
			log::warn!(
				"llm load already in progress; refusing request for {}",
				spec.id
			);
			return Err("a model is already loading - try again in a moment".into());
		}
		let result = self.load_llm_inner(app, spec);
		self.llm_loading.store(false, Ordering::SeqCst);
		self.emit_llm_status(app);
		result
	}

	/// Generic load/swap flow shared by both engine kinds: mark loading,
	/// run `prepare` (backend init for llama), drop the previous engine,
	/// load the new file, and either install it or roll back to the
	/// previous model. Blocking; call from a background thread.
	///
	/// `rollback_on_same`: whisper retries loading the same model after a
	/// transient failure; an identical llama reload fails deterministically
	/// on the same mmap, so it does not.
	/// `missing_when_vanished`: the llm path distinguishes "file deleted
	/// mid-load" (Missing) from a genuine load failure (Error).
	#[allow(clippy::too_many_arguments)]
	fn swap_engine<E>(
		&self,
		app: &AppHandle,
		spec: &ModelSpec,
		status: &std::sync::Mutex<EngineStatus>,
		emit: fn(&Self, &AppHandle),
		slot: fn(&mut Runtime) -> &mut Option<Arc<E>>,
		model_id_of: fn(&E) -> &str,
		prepare: impl FnOnce(&mut Runtime) -> Result<(), String>,
		load: impl Fn(&Path) -> Result<E, String>,
		rollback: impl Fn(&ModelSpec) -> Result<(), String>,
		rollback_on_same: bool,
		missing_when_vanished: bool,
	) -> Result<(), String> {
		{
			let mut s = lock(status);
			*s = EngineStatus::loading(spec.id);
		}
		emit(self, app);

		// App-managed copy first; fall back to a file already present in
		// the user's HuggingFace hub cache (no app copy to create).
		let path = self
			.resolve_model_file(spec)
			.unwrap_or_else(|| self.model_path(spec));
		let prev_spec: Option<ModelSpec> = {
			let mut runtime = lock(&self.runtime);
			slot(&mut runtime)
				.as_ref()
				.map(|engine| model_id_of(engine).to_string())
				.and_then(|id| find_model(&id, spec.kind))
				.cloned()
		};

		// Engine-specific setup (llama backend init), then drop the
		// previous engine so peak memory stays at one model.
		let result = (|| -> Result<E, String> {
			{
				let mut runtime = lock(&self.runtime);
				prepare(&mut runtime)?;
				*slot(&mut runtime) = None;
			}
			load(&path)
		})();
		let loaded = match result {
			Ok(engine) => Some(engine),
			Err(e) => {
				log::error!("{} load failed: {e}", spec.id);
				None
			}
		};

		// Install the new engine, or roll back to the previous one.
		let had_loaded = loaded.is_some();
		let install = loaded.filter(|_| path.is_file());
		match install {
			Some(engine) => {
				let mut runtime = lock(&self.runtime);
				*slot(&mut runtime) = Some(Arc::new(engine));
				drop(runtime);
				let mut s = lock(status);
				*s = EngineStatus::ready(Some(spec.id));
				Ok(())
			}
			None => {
				let vanished = had_loaded;
				if vanished {
					// The engine built fine but the model file vanished
					// while loading; don't resurrect a deleted model.
					log::warn!("{} was deleted while loading; not activating it", spec.id);
				}
				if let Some(prev) = rollback_candidate(prev_spec, spec, rollback_on_same) {
					match rollback(&prev) {
						Ok(()) => {
							log::warn!(
								"switch to {} failed; previous model {} is active again",
								spec.id,
								prev.id
							);
							let mut s = lock(status);
							*s = EngineStatus::new(
								EngineState::Ready,
								Some(prev.id),
								Some(&format!(
									"could not load {0} - {1} is still active",
									spec.id, prev.id
								)),
							);
							return Err(format!(
								"could not load {} - {} is still active",
								spec.id, prev.id
							));
						}
						Err(rollback_err) => {
							log::error!("rollback to {} failed: {rollback_err}", prev.id);
						}
					}
				}
				let mut s = lock(status);
				if vanished && missing_when_vanished {
					*s = EngineStatus::missing();
					return Err(format!("{} was deleted while loading", spec.id));
				}
				*s = EngineStatus::error(Some(spec.id), "model failed to load");
				Err("model failed to load".into())
			}
		}
	}

	fn load_llm_inner(&self, app: &AppHandle, spec: &ModelSpec) -> Result<(), String> {
		self.swap_engine(
			app,
			spec,
			&self.llm_status,
			Self::emit_llm_status,
			|runtime| &mut runtime.llm,
			|engine| &engine.model_id,
			|runtime| {
				// The llama backend is initialized once and kept for the
				// process lifetime; engines come and go on top of it.
				if runtime.backend.is_none() {
					let backend = llama_cpp_2::llama_backend::LlamaBackend::init()
						.map_err(|e| format!("failed to init llama backend: {e}"))?;
					runtime.backend = Some(Arc::new(backend));
				}
				Ok(())
			},
			|path| {
				let backend = lock(&self.runtime)
					.backend
					.clone()
					.ok_or_else(|| "llama backend missing".to_string())?;
				LocalLlm::load(backend, path, spec.id)
			},
			|prev| self.reload_llm(prev),
			LLM_ROLLBACK_ON_SAME,
			true,
		)
	}

	/// Best-effort reload of a previously working model (rollback path).
	fn reload_llm(&self, spec: &ModelSpec) -> Result<(), String> {
		let path = self
			.resolve_model_file(spec)
			.ok_or_else(|| format!("model file for {} is gone", spec.id))?;
		let backend = lock(&self.runtime)
			.backend
			.clone()
			.ok_or_else(|| "llama backend missing".to_string())?;
		let engine = LocalLlm::load(backend, &path, spec.id)?;
		let mut runtime = lock(&self.runtime);
		runtime.llm = Some(Arc::new(engine));
		Ok(())
	}

	/// Load the given whisper model file. Blocking; call from a background
	/// thread. Same staging/rollback contract as `load_llm`.
	pub fn load_stt(&self, app: &AppHandle, spec: &ModelSpec) -> Result<(), String> {
		if self.stt_loading.swap(true, Ordering::SeqCst) {
			log::warn!(
				"stt load already in progress; refusing request for {}",
				spec.id
			);
			return Err("a model is already loading - try again in a moment".into());
		}
		let result = self.load_stt_inner(app, spec);
		self.stt_loading.store(false, Ordering::SeqCst);
		self.emit_stt_status(app);
		result
	}

	fn load_stt_inner(&self, app: &AppHandle, spec: &ModelSpec) -> Result<(), String> {
		self.swap_engine(
			app,
			spec,
			&self.stt_status,
			Self::emit_stt_status,
			|runtime| &mut runtime.stt,
			|engine| &engine.model_id,
			|_runtime| Ok(()),
			|path| SttEngine::load(path, spec.id),
			|prev| {
				let prev_path = self
					.resolve_model_file(prev)
					.ok_or_else(|| format!("model file for {} is gone", prev.id))?;
				{}
				let engine = SttEngine::load(&prev_path, prev.id)?;
				lock(&self.runtime).stt = Some(Arc::new(engine));
				Ok(())
			},
			STT_ROLLBACK_ON_SAME,
			false,
		)
	}
}

#[cfg(test)]
mod rollback_tests {
	use super::super::catalog::{LLM_MODELS, STT_MODELS};
	use super::{rollback_candidate, LLM_ROLLBACK_ON_SAME, STT_ROLLBACK_ON_SAME};

	#[test]
	fn whisper_retries_the_same_model_llama_does_not() {
		let small = &STT_MODELS[1];
		let tiny = &STT_MODELS[2];
		// a failed reload of the active whisper model gets a retry
		// (339b19e): its engine was already dropped
		let retry = rollback_candidate(Some(small.clone()), small, STT_ROLLBACK_ON_SAME);
		assert_eq!(retry.map(|s| s.id), Some(small.id));
		// switching whisper models rolls back to the previous one
		let back = rollback_candidate(Some(tiny.clone()), small, STT_ROLLBACK_ON_SAME);
		assert_eq!(back.map(|s| s.id), Some(tiny.id));

		// llama: an identical reload fails the same way, so no retry...
		let gemma = &LLM_MODELS[0];
		let other = &LLM_MODELS[1];
		assert!(rollback_candidate(Some(gemma.clone()), gemma, LLM_ROLLBACK_ON_SAME).is_none());
		// ...but a failed switch still restores the previous model
		let back = rollback_candidate(Some(other.clone()), gemma, LLM_ROLLBACK_ON_SAME);
		assert_eq!(back.map(|s| s.id), Some(other.id));

		// nothing was loaded before: nothing to restore
		assert!(rollback_candidate(None, small, STT_ROLLBACK_ON_SAME).is_none());
	}
}

#[cfg(test)]
mod settings_cache_tests {
	use super::super::catalog::LLM_MODELS;
	use super::*;
	#[test]
	fn ai_settings_cache_round_trips_through_save() {
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join("t.db")).expect("db");
		let state = AppState::new(db, dir.path().to_path_buf());

		// cold read loads and warms the cache
		assert_eq!(state.ai_settings().llm_model, LLM_MODELS[0].id);
		// a write through save_ai_settings refreshes the cache
		let mut next = state.ai_settings();
		next.stt_language = "fr-FR".into();
		state.save_ai_settings(&next).expect("save");
		assert_eq!(state.ai_settings().stt_language, "fr-FR");
		// and persisted: a fresh AppState sees the same value
		let db2 = Db::open(&dir.path().join("t.db")).expect("reopen db");
		let state2 = AppState::new(db2, dir.path().to_path_buf());
		assert_eq!(state2.ai_settings().stt_language, "fr-FR");
	}
	#[test]
	fn ai_settings_cache_is_a_cache_not_a_source() {
		// direct DB writes (the legacy path) are visible after a cache
		// refresh via save, proving the cache never outruns the database
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join("t.db")).expect("db");
		let state = AppState::new(db, dir.path().to_path_buf());
		let _ = state.ai_settings(); // warm
		let mut updated = state.ai_settings();
		updated.llm_model = "gemma-4-E4B".into();
		state.save_ai_settings(&updated).expect("save");
		assert_eq!(state.ai_settings().llm_model, "gemma-4-E4B");
	}
}
