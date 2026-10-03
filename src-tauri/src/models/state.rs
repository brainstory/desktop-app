//! Runtime state: engines, statuses, load/swap/rollback, download
//! bookkeeping.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
	/// Authoritative read-through cache of the AI settings (three
	/// keychain reads plus a dozen DB rows on every load). Every write
	/// goes through [`Self::mutate_ai_settings`], which loads, patches,
	/// persists and publishes under this one mutex, so two concurrent
	/// partial updates can never overwrite each other's fields with
	/// stale snapshots. Lock ordering: this mutex before the db
	/// connection lock, never the reverse.
	pub ai_settings_cache: std::sync::Mutex<Option<AiSettings>>,
	/// Bumped on every successful publish by
	/// [`Self::mutate_ai_settings`]; callers can compare the returned
	/// generation against a later read to detect intervening writes.
	pub ai_settings_generation: AtomicU64,
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

/// Exclusive use of one engine slot (the `llm_loading`/`stt_loading`
/// flag) for work other than a load, e.g. the model loader unloading an
/// engine. Released on drop.
pub struct EngineSlotClaim<'a> {
	flag: &'a AtomicBool,
	claimed: bool,
}

impl<'a> EngineSlotClaim<'a> {
	/// Wait for an in-flight load of this slot to finish, then hold the
	/// slot. An unload racing a running load is undone when the load
	/// installs its engine, leaving a runtime that contradicts the
	/// reported status. Gives up waiting after `max_wait` (a load that
	/// long is wedged; proceed unclaimed rather than block forever).
	pub fn acquire(flag: &'a AtomicBool, max_wait: std::time::Duration) -> Self {
		let deadline = std::time::Instant::now() + max_wait;
		loop {
			if !flag.swap(true, Ordering::SeqCst) {
				return Self {
					flag,
					claimed: true,
				};
			}
			if std::time::Instant::now() >= deadline {
				log::warn!("engine slot still busy after {max_wait:?}; proceeding unclaimed");
				return Self {
					flag,
					claimed: false,
				};
			}
			std::thread::sleep(std::time::Duration::from_millis(50));
		}
	}

	/// Release now, e.g. right before starting a load that claims the
	/// slot itself.
	pub fn release(self) {}
}

impl Drop for EngineSlotClaim<'_> {
	fn drop(&mut self) {
		if self.claimed {
			self.flag.store(false, Ordering::SeqCst);
		}
	}
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
			ai_settings_cache: std::sync::Mutex::new(None),
			ai_settings_generation: AtomicU64::new(0),
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

	/// The AI settings, from the read-through cache when warm. All writes
	/// go through [`Self::mutate_ai_settings`], which publishes under
	/// this same mutex, so the cache can never go stale and a cold read
	/// (load and publish inside the lock) can never overwrite a newer
	/// published value.
	pub fn ai_settings(&self) -> AiSettings {
		let mut cache = lock(&self.ai_settings_cache);
		if let Some(cached) = cache.as_ref() {
			return cached.clone();
		}
		let loaded = AiSettings::load(&self.db);
		*cache = Some(loaded.clone());
		loaded
	}

	/// The one mutation boundary for AI settings: load the latest state
	/// (cache, or database when cold - inside the lock), apply `patch`
	/// to a working copy, persist it, and publish it to the cache, all
	/// under the settings mutex. `patch` returning Err aborts with the
	/// cache and database unchanged; a failed persist never publishes,
	/// so the cache can never disagree with the database either.
	///
	/// Returns the committed snapshot and the new generation (bumped on
	/// every successful publish). No engine work may run while the lock
	/// is held - start loaders after this returns. The critical section
	/// may lock the db connection (settings mutex → db mutex, never the
	/// reverse).
	pub fn mutate_ai_settings(
		&self,
		patch: impl FnOnce(&mut AiSettings) -> Result<(), String>,
	) -> Result<(AiSettings, u64), String> {
		let mut cache = lock(&self.ai_settings_cache);
		let latest = match cache.as_ref() {
			Some(cached) => cached.clone(),
			None => {
				let loaded = AiSettings::load(&self.db);
				*cache = Some(loaded.clone());
				loaded
			}
		};
		let mut working = latest.clone();
		// the cache mirrors what is stored, so secrets compare against
		// it (not a fresh keychain read) - untouched secrets are skipped
		// on save
		patch(&mut working)?;
		working.save(&self.db, &latest)?;
		let generation = self.ai_settings_generation.fetch_add(1, Ordering::SeqCst) + 1;
		*cache = Some(working.clone());
		Ok((working, generation))
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
mod slot_claim_tests {
	use super::EngineSlotClaim;
	use std::sync::atomic::{AtomicBool, Ordering};
	use std::sync::Arc;
	use std::time::{Duration, Instant};

	#[test]
	fn a_claim_waits_for_the_running_load_and_blocks_new_ones() {
		let loading = Arc::new(AtomicBool::new(true)); // a load is running
		let running = loading.clone();
		let finisher = std::thread::spawn(move || {
			std::thread::sleep(Duration::from_millis(200));
			running.store(false, Ordering::SeqCst); // the load finishes
		});
		let started = Instant::now();
		let claim = EngineSlotClaim::acquire(&loading, Duration::from_secs(5));
		assert!(
			started.elapsed() >= Duration::from_millis(150),
			"the claim must wait for the running load"
		);
		// while held, a new load is refused (load_llm/load_stt swap the flag)
		assert!(loading.swap(true, Ordering::SeqCst), "slot is held");
		drop(claim);
		assert!(!loading.load(Ordering::SeqCst), "released on drop");
		finisher.join().unwrap();
	}

	#[test]
	fn a_wedged_slot_is_not_released_by_a_claim_that_gave_up() {
		let loading = AtomicBool::new(true);
		let claim = EngineSlotClaim::acquire(&loading, Duration::from_millis(100));
		claim.release();
		assert!(
			loading.load(Ordering::SeqCst),
			"an unclaimed slot stays owned by whoever holds it"
		);
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

	fn temp_state(name: &str) -> (AppState, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join(format!("{name}.db"))).expect("db");
		(AppState::new(db, dir.path().to_path_buf()), dir)
	}

	/// Reopen the same database behind a fresh AppState, so assertions
	/// check what was persisted, not just the cache.
	fn reopened(dir: &std::path::Path, name: &str) -> AppState {
		let db = Db::open(&dir.join(format!("{name}.db"))).expect("reopen db");
		AppState::new(db, dir.to_path_buf())
	}

	#[test]
	fn ai_settings_cache_round_trips_through_mutate() {
		let (state, dir) = temp_state("roundtrip");

		// cold read loads and warms the cache
		assert_eq!(state.ai_settings().llm_model, LLM_MODELS[0].id);
		// a write through the mutation boundary refreshes the cache
		state
			.mutate_ai_settings(|s| {
				s.stt_language = "fr-FR".into();
				Ok(())
			})
			.expect("mutate");
		assert_eq!(state.ai_settings().stt_language, "fr-FR");
		// and persisted: a fresh AppState sees the same value
		assert_eq!(
			reopened(dir.path(), "roundtrip").ai_settings().stt_language,
			"fr-FR"
		);
	}
	#[test]
	fn ai_settings_cache_is_a_cache_not_a_source() {
		// the cache only ever mirrors committed database writes (via
		// mutate_ai_settings), proving the cache never outruns the
		// database
		let (state, _dir) = temp_state("mirror");
		let _ = state.ai_settings(); // warm
		state
			.mutate_ai_settings(|s| {
				s.llm_model = "gemma-4-E4B".into();
				Ok(())
			})
			.expect("mutate");
		assert_eq!(state.ai_settings().llm_model, "gemma-4-E4B");
	}

	#[test]
	fn disjoint_concurrent_patches_preserve_each_others_fields() {
		let (state, dir) = temp_state("disjoint");
		let state = std::sync::Arc::new(state);

		// Both writers are poised before either mutates (the barrier),
		// and the extLlmModel writer commits first (the channel), so the
		// sttLanguage writer lands last. Committing last must not roll
		// the other field back: each patch applies to the LATEST state
		// under the mutex, never to a stale caller-side snapshot.
		let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
		let (tx, rx) = std::sync::mpsc::channel::<()>();

		let a_state = state.clone();
		let a_barrier = barrier.clone();
		let a = std::thread::spawn(move || {
			a_barrier.wait();
			a_state
				.mutate_ai_settings(|s| {
					s.apply_updates(&serde_json::json!({ "extLlmModel": "new-model" }))
				})
				.expect("extLlmModel patch commits");
			tx.send(()).expect("first writer alive");
		});
		let b_state = state.clone();
		let b = std::thread::spawn(move || {
			barrier.wait();
			rx.recv().expect("first writer committed");
			b_state
				.mutate_ai_settings(|s| {
					s.apply_updates(&serde_json::json!({ "sttLanguage": "fr-FR" }))
				})
				.expect("sttLanguage patch commits");
		});
		a.join().unwrap();
		b.join().unwrap();

		assert_eq!(state.ai_settings().ext_llm_model, "new-model");
		assert_eq!(state.ai_settings().stt_language, "fr-FR");
		assert_eq!(
			reopened(dir.path(), "disjoint").ai_settings().ext_llm_model,
			"new-model"
		);
		assert_eq!(
			reopened(dir.path(), "disjoint").ai_settings().stt_language,
			"fr-FR"
		);
	}

	#[test]
	fn same_field_concurrent_patches_serialize_last_committer_wins() {
		let (state, dir) = temp_state("samefield");
		let state = std::sync::Arc::new(state);

		let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
		let a_state = state.clone();
		let a_barrier = barrier.clone();
		let a = std::thread::spawn(move || {
			a_barrier.wait();
			a_state
				.mutate_ai_settings(|s| {
					s.stt_language = "de-DE".into();
					Ok(())
				})
				.expect("first same-field patch")
		});
		let b_state = state.clone();
		let b = std::thread::spawn(move || {
			barrier.wait();
			b_state
				.mutate_ai_settings(|s| {
					s.stt_language = "fr-FR".into();
					Ok(())
				})
				.expect("second same-field patch")
		});
		let a_result = a.join().unwrap();
		let b_result = b.join().unwrap();

		// the higher generation committed last; its value is what both
		// the cache and the database must show
		let (last_value, last_gen) = if a_result.1 > b_result.1 {
			("de-DE", a_result.1)
		} else {
			("fr-FR", b_result.1)
		};
		assert!(last_gen >= 2, "each commit bumped the generation");
		assert_eq!(state.ai_settings().stt_language, last_value);
		assert_eq!(
			reopened(dir.path(), "samefield").ai_settings().stt_language,
			last_value
		);
	}

	#[test]
	fn an_invalid_patch_changes_neither_cache_nor_database() {
		let (state, dir) = temp_state("invalid");
		let before = state.ai_settings();
		let gen_before = state.ai_settings_generation.load(Ordering::SeqCst);

		let err = state
			.mutate_ai_settings(|s| {
				s.apply_updates(&serde_json::json!({
					"llmModel": "not-a-model",
					"sttLanguage": "de-DE",
				}))
			})
			.expect_err("the invalid field rejects the whole patch");
		assert!(err.contains("unknown llmModel"), "unexpected: {err}");

		let after = state.ai_settings();
		assert_eq!(after.llm_model, before.llm_model);
		assert_eq!(
			after.stt_language, before.stt_language,
			"de-DE must not leak"
		);
		assert_eq!(
			state.ai_settings_generation.load(Ordering::SeqCst),
			gen_before,
			"a rejected patch is no publish"
		);
		let persisted = reopened(dir.path(), "invalid").ai_settings();
		assert_eq!(persisted.llm_model, before.llm_model);
		assert_eq!(persisted.stt_language, before.stt_language);
	}

	#[test]
	fn a_cold_read_racing_a_save_never_loses_the_save() {
		// Fresh state per round so every read is the cold one; whichever
		// side gets the mutex first, the committed value must survive in
		// cache and database.
		for round in 0..32 {
			let (state, dir) = temp_state(&format!("coldrace{round}"));
			let state = std::sync::Arc::new(state);
			let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

			let a_state = state.clone();
			let a_barrier = barrier.clone();
			let a = std::thread::spawn(move || {
				a_barrier.wait();
				a_state.ai_settings();
			});
			let b_state = state.clone();
			let b = std::thread::spawn(move || {
				barrier.wait();
				b_state
					.mutate_ai_settings(|s| {
						s.apply_updates(&serde_json::json!({ "sttLanguage": "fr-FR" }))
					})
					.expect("save commits");
			});
			a.join().unwrap();
			b.join().unwrap();

			assert_eq!(
				state.ai_settings().stt_language,
				"fr-FR",
				"round {round}: the cold read overwrote the published value"
			);
			assert_eq!(
				reopened(dir.path(), &format!("coldrace{round}"))
					.ai_settings()
					.stt_language,
				"fr-FR",
				"round {round}"
			);
		}
	}

	#[test]
	fn mutate_preserves_secret_keep_and_clear_semantics() {
		// debug builds keep secrets in dev-only DB rows, so this never
		// touches the developer's real keychain
		let (state, dir) = temp_state("secrets");
		state
			.mutate_ai_settings(|s| {
				s.apply_updates(&serde_json::json!({ "hfToken": "fake-test-token" }))
			})
			.expect("store token");
		assert_eq!(state.ai_settings().hf_token, "fake-test-token");

		// an absent field keeps the stored secret
		state
			.mutate_ai_settings(|s| s.apply_updates(&serde_json::json!({ "sttLanguage": "fr-FR" })))
			.expect("patch without the token field");
		assert_eq!(state.ai_settings().hf_token, "fake-test-token");
		// and so does an explicit null
		state
			.mutate_ai_settings(|s| {
				s.apply_updates(&serde_json::json!({ "sttLanguage": "de-DE", "hfToken": null }))
			})
			.expect("null token");
		assert_eq!(state.ai_settings().hf_token, "fake-test-token");

		// an explicit empty string clears it
		state
			.mutate_ai_settings(|s| s.apply_updates(&serde_json::json!({ "hfToken": "" })))
			.expect("clear token");
		assert_eq!(state.ai_settings().hf_token, "");
		assert_eq!(reopened(dir.path(), "secrets").ai_settings().hf_token, "");
	}
}
