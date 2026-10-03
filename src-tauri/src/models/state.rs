//! Runtime state: engines, statuses, load/swap/rollback, download
//! bookkeeping.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tauri::AppHandle;

use super::ai_settings::AiSettings;
use super::catalog::{find_model, ModelKind, ModelSpec};
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
}

impl<'a> EngineSlotClaim<'a> {
	/// Wait for an in-flight load of this slot to finish, then hold the
	/// slot. An unload racing a running load is undone when the load
	/// installs its engine, leaving a runtime that contradicts the
	/// reported status. Returns None after `max_wait` (a load that long
	/// is wedged): a timed-out caller must ABORT - mutating without the
	/// claim would race whatever still holds the slot, so the holder
	/// keeps the slot and the caller leaves the runtime untouched.
	pub fn acquire(flag: &'a AtomicBool, max_wait: std::time::Duration) -> Option<Self> {
		let deadline = std::time::Instant::now() + max_wait;
		loop {
			if !flag.swap(true, Ordering::SeqCst) {
				return Some(Self { flag });
			}
			if std::time::Instant::now() >= deadline {
				log::warn!("engine slot still busy after {max_wait:?}; aborting without the claim");
				return None;
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
		self.flag.store(false, Ordering::SeqCst);
	}
}

/// One engine kind's home in the runtime, so the generic load/swap
/// flow stays independent of the engine type (and testable against a
/// stand-in slot with fake engines).
pub(crate) trait EngineSlot<E> {
	/// The resident engine, if any.
	fn installed(&self) -> Option<Arc<E>>;
	/// Drop the resident engine (peak memory stays at one model).
	fn vacate(&self);
	/// Make `engine` the resident one.
	fn install(&self, engine: Arc<E>);
}

/// The whisper slot: `runtime.stt` behind its lock.
pub(crate) struct SttSlot<'a>(pub &'a AppState);

impl EngineSlot<SttEngine> for SttSlot<'_> {
	fn installed(&self) -> Option<Arc<SttEngine>> {
		lock(&self.0.runtime).stt.clone()
	}
	fn vacate(&self) {
		lock(&self.0.runtime).stt = None;
	}
	fn install(&self, engine: Arc<SttEngine>) {
		lock(&self.0.runtime).stt = Some(engine);
	}
}

/// The local LLM slot: `runtime.llm` behind its lock.
pub(crate) struct LlmSlot<'a>(pub &'a AppState);

impl EngineSlot<LocalLlm> for LlmSlot<'_> {
	fn installed(&self) -> Option<Arc<LocalLlm>> {
		lock(&self.0.runtime).llm.clone()
	}
	fn vacate(&self) {
		lock(&self.0.runtime).llm = None;
	}
	fn install(&self, engine: Arc<LocalLlm>) {
		lock(&self.0.runtime).llm = Some(engine);
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
	/// cache and database unchanged.
	///
	/// Persistence is staged (settings row, then each changed secret),
	/// and SQLite cannot transact the OS keychain, so a failed persist
	/// may still have committed part of the update. On any save
	/// failure the cache is therefore re-published from
	/// [`AiSettings::reload`] - the state the stores actually hold -
	/// and the returned Err names the failing stage/secret (never a
	/// secret value). A retry goes through here again and builds on
	/// that reloaded state, so fields committed by the failed attempt
	/// are preserved, never reverted. If the reload itself fails the
	/// cache is dropped (degraded mode: the next read loads cold) and
	/// the Err says the settings state is unavailable - never success
	/// after an incomplete write, never a claimed rollback of
	/// keychain mutations.
	///
	/// Returns the committed snapshot and the new generation (bumped
	/// on every successful publish; a failure-path reconcile publish
	/// does NOT bump it - it reports the stores, it is not a new
	/// commit, and the Err already tells callers to re-read). No
	/// engine work may run while the lock is held - start loaders
	/// after this returns. The critical section may lock the db
	/// connection (settings mutex → db mutex, never the reverse).
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
		match working.save(&self.db, &latest) {
			Ok(()) => {
				let generation = self.ai_settings_generation.fetch_add(1, Ordering::SeqCst) + 1;
				*cache = Some(working.clone());
				Ok((working, generation))
			}
			Err(failure) => {
				// The failed save may have committed (the row
				// transaction runs before the secret writes), so the
				// pre-save snapshot is obsolete: publish the
				// authoritative reload so the cache can never serve
				// values the stores no longer hold.
				match AiSettings::reload(&self.db) {
					Ok(authoritative) => {
						*cache = Some(authoritative);
						Err(failure.to_string())
					}
					Err(degraded) => {
						// The store cannot even be read back: drop the
						// cache so the next read loads cold, and say
						// so instead of republishing unverified values.
						*cache = None;
						Err(format!(
							"settings state unavailable after a failed save: {degraded} (failed save: {failure})"
						))
					}
				}
			}
		}
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
		if cfg!(test) {
			// The hub cache is shared with other tools and may hold the
			// developer's real models: consulting it would make catalog
			// decisions (and these tests) machine-dependent. Tests only
			// ever see the app-managed copy in their temp dirs.
			return None;
		}
		hf_hub_cache_candidates()
			.iter()
			.find_map(|cache| hf_cache_model_path(cache, spec))
	}

	pub fn is_model_downloaded(&self, spec: &ModelSpec) -> bool {
		self.resolve_model_file(spec).is_some()
	}

	pub fn emit_llm_status(&self, app: &AppHandle) {
		self.emit_llm_status_events(&crate::AppStatusEvents(app));
	}

	/// [`Self::emit_llm_status`] over an injectable event sink.
	pub(crate) fn emit_llm_status_events(&self, events: &dyn crate::StatusEvents) {
		let status = lock(&self.llm_status).clone();
		events.llm_status(&status);
	}

	pub fn emit_stt_status(&self, app: &AppHandle) {
		self.emit_stt_status_events(&crate::AppStatusEvents(app));
	}

	/// [`Self::emit_stt_status`] over an injectable event sink.
	pub(crate) fn emit_stt_status_events(&self, events: &dyn crate::StatusEvents) {
		let status = lock(&self.stt_status).clone();
		events.stt_status(&status);
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
		// An explicit activation acts on the newest committed settings at
		// the moment it starts; a save landing while it loads makes this
		// load stale under the slot claim, so it aborts rather than
		// install an engine the newer decision superseded.
		let generation = self.ai_settings_generation.load(Ordering::SeqCst);
		self.load_llm_events(&crate::AppStatusEvents(app), spec, generation)
	}

	/// [`Self::load_llm`] over an injectable event sink, decided at the
	/// given settings generation (how the loader and the post-download
	/// auto-load pass their captured generation).
	pub(crate) fn load_llm_events(
		&self,
		events: &dyn crate::StatusEvents,
		spec: &ModelSpec,
		generation: u64,
	) -> Result<(), String> {
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
		let result = self.load_llm_claimed(events, spec, generation);
		self.llm_loading.store(false, Ordering::SeqCst);
		self.emit_llm_status_events(events);
		result
	}

	fn load_llm_claimed(
		&self,
		events: &dyn crate::StatusEvents,
		spec: &ModelSpec,
		generation: u64,
	) -> Result<(), String> {
		// Newer settings were committed since this load was decided (a
		// save, an activation): refuse under the claim, before any
		// status change or engine work, so the newer decision's loader
		// owns the outcome. This is the re-verification that closes the
		// loader's release-and-reacquire interval.
		if self.ai_settings_generation.load(Ordering::SeqCst) > generation {
			log::info!(
				"llm load of {0} skipped: newer settings were committed",
				spec.id
			);
			return Err(
				"settings changed while the model was loading - it was not activated".into(),
			);
		}
		self.swap_engine(
			events,
			spec,
			&self.llm_status,
			crate::notify_llm,
			&LlmSlot(self),
			|engine| engine.model_id.as_str(),
			|| {
				// The llama backend is initialized once and kept for the
				// process lifetime; engines come and go on top of it.
				let mut runtime = lock(&self.runtime);
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
			|| self.ai_settings_generation.load(Ordering::SeqCst) <= generation,
		)
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
	/// `fresh`: re-checked under the slot claim right before installing,
	/// publishing or rolling back - a load that finished after newer
	/// settings committed is obsolete and must touch none of them.
	///
	/// The slot and engine construction are parameters (the fake-engine
	/// seam): the production callers pass the real runtime slots and
	/// constructors, tests pass stand-ins.
	#[allow(clippy::too_many_arguments)]
	fn swap_engine<E>(
		&self,
		events: &dyn crate::StatusEvents,
		spec: &ModelSpec,
		status: &std::sync::Mutex<EngineStatus>,
		notify: fn(&EngineStatus, &dyn crate::StatusEvents),
		slot: &dyn EngineSlot<E>,
		model_id_of: fn(&E) -> &str,
		prepare: impl FnOnce() -> Result<(), String>,
		load: impl Fn(&Path) -> Result<E, String>,
		rollback: impl Fn(&ModelSpec) -> Result<(), String>,
		rollback_on_same: bool,
		missing_when_vanished: bool,
		fresh: impl Fn() -> bool,
	) -> Result<(), String> {
		// The wanted engine is already resident under this claim: nothing
		// to load and nothing to publish - e.g. a same-model activation
		// or a save that changed nothing about routing must not reload a
		// working engine (or blip its status through loading).
		if slot.installed().as_ref().map(|engine| model_id_of(engine)) == Some(spec.id) {
			return Ok(());
		}
		{
			let mut s = lock(status);
			*s = EngineStatus::loading(spec.id);
		}
		notify(&lock(status).clone(), events);

		// App-managed copy first; fall back to a file already present in
		// the user's HuggingFace hub cache (no app copy to create).
		let path = self
			.resolve_model_file(spec)
			.unwrap_or_else(|| self.model_path(spec));
		let prev_spec: Option<ModelSpec> = slot
			.installed()
			.as_ref()
			.map(|engine| model_id_of(engine).to_string())
			.and_then(|id| find_model(&id, spec.kind))
			.cloned();

		// Engine-specific setup (llama backend init), then drop the
		// previous engine so peak memory stays at one model.
		let result = (|| -> Result<E, String> {
			prepare()?;
			slot.vacate();
			load(&path)
		})();
		let loaded = match result {
			Ok(engine) => Some(engine),
			Err(e) => {
				log::error!("{} load failed: {e}", spec.id);
				None
			}
		};

		// Re-check under the claim (held since before the engine work),
		// immediately before installing, publishing or rolling back: a
		// settings save that committed while the file loaded makes this
		// whole outcome obsolete. Install nothing, publish nothing, do
		// not touch rollback - the newer decision owns the outcome.
		if !fresh() {
			log::info!(
				"{} finished loading after newer settings were saved; installing nothing",
				spec.id
			);
			return Err(
				"settings changed while the model was loading - it was not activated".into(),
			);
		}

		// Install the new engine, or roll back to the previous one.
		let had_loaded = loaded.is_some();
		let install = loaded.filter(|_| path.is_file());
		match install {
			Some(engine) => {
				slot.install(Arc::new(engine));
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
	/// thread. Same staging/rollback contract as [`Self::load_llm`],
	/// including the at-entry settings generation.
	pub fn load_stt(&self, app: &AppHandle, spec: &ModelSpec) -> Result<(), String> {
		let generation = self.ai_settings_generation.load(Ordering::SeqCst);
		self.load_stt_events(&crate::AppStatusEvents(app), spec, generation)
	}

	/// [`Self::load_stt`] over an injectable event sink, decided at the
	/// given settings generation.
	pub(crate) fn load_stt_events(
		&self,
		events: &dyn crate::StatusEvents,
		spec: &ModelSpec,
		generation: u64,
	) -> Result<(), String> {
		if self.stt_loading.swap(true, Ordering::SeqCst) {
			log::warn!(
				"stt load already in progress; refusing request for {}",
				spec.id
			);
			return Err("a model is already loading - try again in a moment".into());
		}
		let result = self.load_stt_claimed(events, spec, generation);
		self.stt_loading.store(false, Ordering::SeqCst);
		self.emit_stt_status_events(events);
		result
	}

	fn load_stt_claimed(
		&self,
		events: &dyn crate::StatusEvents,
		spec: &ModelSpec,
		generation: u64,
	) -> Result<(), String> {
		// Same under-the-claim re-verification as the llm path above.
		if self.ai_settings_generation.load(Ordering::SeqCst) > generation {
			log::info!(
				"stt load of {0} skipped: newer settings were committed",
				spec.id
			);
			return Err(
				"settings changed while the model was loading - it was not activated".into(),
			);
		}
		self.swap_engine(
			events,
			spec,
			&self.stt_status,
			crate::notify_stt,
			&SttSlot(self),
			|engine| engine.model_id.as_str(),
			|| Ok(()),
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
			|| self.ai_settings_generation.load(Ordering::SeqCst) <= generation,
		)
	}

	/// The production delete entry: [`Self::delete_model_exclusive`]
	/// against the real runtime slots and statuses for `spec`'s kind,
	/// with the download guard (`blocked`) and the file deletion
	/// (`delete_files`) injected by the caller - models_cmd owns the
	/// download bookkeeping and the hub-cache candidate list.
	pub(crate) fn delete_model_kind_events(
		&self,
		events: &dyn crate::StatusEvents,
		spec: &ModelSpec,
		blocked: &dyn Fn() -> Option<&'static str>,
		delete_files: &dyn Fn() -> Result<bool, String>,
		slot_wait: std::time::Duration,
	) -> Result<(), String> {
		match spec.kind {
			ModelKind::Llm => self.delete_model_exclusive(
				events,
				spec,
				&self.llm_status,
				crate::notify_llm,
				&LlmSlot(self),
				|engine| engine.model_id.as_str(),
				blocked,
				delete_files,
				slot_wait,
			),
			ModelKind::Stt => self.delete_model_exclusive(
				events,
				spec,
				&self.stt_status,
				crate::notify_stt,
				&SttSlot(self),
				|engine| engine.model_id.as_str(),
				blocked,
				delete_files,
				slot_wait,
			),
		}
	}

	/// Delete `spec`'s files and unload its engine as ONE exclusive
	/// operation on the model's engine slot:
	///
	/// 1. Acquire the [`EngineSlotClaim`] for `spec`'s kind. Loads take
	///    the same claim, so a load/activate can neither start
	///    mid-delete nor race one; an in-flight load is waited out, and
	///    an acquire that times out ABORTS with a busy error, mutating
	///    nothing (the holder keeps its slot).
	/// 2. Re-check `blocked` UNDER the claim: a download may have
	///    registered while this delete waited out a load, and deleting
	///    then would pull the file out from under the transfer.
	/// 3. Unload the resident engine when it is `spec` (a loaded engine
	///    mmaps the model, and on Windows an open mapping makes
	///    remove_file fail).
	/// 4. Run `delete_files` (the app copy and the hub-cache
	///    candidates, with the conservative shared-blob semantics).
	/// 5. Publish the resulting status - all still under the claim.
	///
	/// Truthful on every exit: once this operation unloaded the
	/// engine, ANY `delete_files` failure - and a file that is already
	/// gone - publishes a non-ready status (missing, or error naming
	/// the failure) BEFORE the Err returns; status never says ready
	/// for an engine the delete just unloaded. Active inference keeps
	/// its `Arc` references: on POSIX unlinking a mapped file
	/// succeeds; where a platform refuses (a Windows mmap), the
	/// removal error is reported, never swallowed.
	///
	/// The slot and the engine-id accessor are parameters (the
	/// fake-engine seam), exactly like [`Self::swap_engine`].
	#[allow(clippy::too_many_arguments)]
	pub(crate) fn delete_model_exclusive<E>(
		&self,
		events: &dyn crate::StatusEvents,
		spec: &ModelSpec,
		status: &std::sync::Mutex<EngineStatus>,
		notify: fn(&EngineStatus, &dyn crate::StatusEvents),
		slot: &dyn EngineSlot<E>,
		model_id_of: fn(&E) -> &str,
		blocked: &dyn Fn() -> Option<&'static str>,
		delete_files: &dyn Fn() -> Result<bool, String>,
		slot_wait: std::time::Duration,
	) -> Result<(), String> {
		// One exclusive operation on the model's engine slot: loads take
		// the same claim, so a load/activate can neither start
		// mid-delete nor be raced by one. A timed-out acquire ABORTS
		// with a busy error and mutates nothing (the holder keeps its
		// slot) - never proceed unclaimed.
		let flag = match spec.kind {
			ModelKind::Llm => &self.llm_loading,
			ModelKind::Stt => &self.stt_loading,
		};
		let Some(_claim) = EngineSlotClaim::acquire(flag, slot_wait) else {
			return Err("model is currently loading - try again in a moment".into());
		};
		// Guards re-checked UNDER the claim: a download may have
		// registered while this delete waited out an in-flight load.
		if let Some(reason) = blocked() {
			return Err(reason.into());
		}
		// Unload BEFORE deleting the files: a loaded engine mmaps the
		// model, and on Windows an open mapping makes remove_file fail.
		// Active inference keeps its Arc references: on POSIX the
		// unlink of a mapped file succeeds; a platform that refuses
		// reports the removal error below - never swallowed.
		let was_resident =
			slot.installed().as_ref().map(|engine| model_id_of(engine)) == Some(spec.id);
		if was_resident {
			slot.vacate();
		}
		let outcome = delete_files();
		if was_resident {
			// The slot is empty now: publish BEFORE any Err return, and
			// never leave a ready status behind for an engine this
			// operation just unloaded.
			let published = match &outcome {
				Ok(_) => EngineStatus::missing(),
				Err(e) => EngineStatus::error(Some(spec.id), e),
			};
			*lock(status) = published;
			notify(&lock(status).clone(), events);
		}
		match outcome {
			Ok(true) => Ok(()),
			Ok(false) => Err("model file not found".into()),
			Err(e) => Err(e),
		}
	}
}

#[cfg(test)]
mod swap_flow_tests {
	use super::{AppState, EngineSlot, EngineState, EngineStatus};
	use crate::StatusEvents;
	use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
	use std::sync::{mpsc, Arc, Mutex};

	/// A stand-in engine: nothing whisper/llama-shaped, just the id the
	/// swap flow reads.
	struct FakeEngine {
		id: String,
	}

	/// A stand-in engine slot: what swap_engine installs into.
	struct FakeSlot(Mutex<Option<Arc<FakeEngine>>>);

	impl FakeSlot {
		fn empty() -> Self {
			Self(Mutex::new(None))
		}
		fn with(engine: FakeEngine) -> Self {
			Self(Mutex::new(Some(Arc::new(engine))))
		}
	}

	impl EngineSlot<FakeEngine> for FakeSlot {
		fn installed(&self) -> Option<Arc<FakeEngine>> {
			self.0.lock().unwrap().clone()
		}
		fn vacate(&self) {
			*self.0.lock().unwrap() = None;
		}
		fn install(&self, engine: Arc<FakeEngine>) {
			*self.0.lock().unwrap() = Some(engine);
		}
	}

	struct RecordingEvents(Mutex<Vec<String>>);

	impl RecordingEvents {
		fn new() -> Self {
			Self(Mutex::new(Vec::new()))
		}
		fn snapshot(&self) -> Vec<String> {
			self.0.lock().unwrap().clone()
		}
	}

	impl StatusEvents for RecordingEvents {
		fn llm_status(&self, status: &EngineStatus) {
			self.0
				.lock()
				.unwrap()
				.push(format!("llm:{:?}:{:?}", status.state, status.model_id));
		}
		fn stt_status(&self, status: &EngineStatus) {
			self.0
				.lock()
				.unwrap()
				.push(format!("stt:{:?}:{:?}", status.state, status.model_id));
		}
	}

	fn temp_state(name: &str) -> (AppState, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::create_dir_all(dir.path().join("models")).expect("models dir");
		let db = crate::db::Db::open(&dir.path().join(format!("{name}.db"))).expect("db");
		(AppState::new(db, dir.path().to_path_buf()), dir)
	}

	fn stt_spec(id: &str) -> super::super::catalog::ModelSpec {
		super::super::catalog::find_model(id, super::super::catalog::ModelKind::Stt)
			.expect("catalog model")
			.clone()
	}

	fn model_id_of(engine: &FakeEngine) -> &str {
		&engine.id
	}

	#[test]
	fn a_fresh_load_installs_and_publishes_ready() {
		let (state, _dir) = temp_state("swap-fresh");
		let spec = stt_spec("whisper-small-en");
		// the install step checks the file still exists on disk
		std::fs::write(state.model_path(&spec), b"stub").expect("model file");
		let slot = FakeSlot::empty();
		let status = Mutex::new(EngineStatus::missing());
		let events = RecordingEvents::new();

		state
			.swap_engine(
				&events,
				&spec,
				&status,
				crate::notify_stt,
				&slot,
				model_id_of,
				|| Ok(()),
				|_path| {
					Ok(FakeEngine {
						id: spec.id.to_string(),
					})
				},
				|_prev| panic!("a successful load never rolls back"),
				true,
				false,
				|| true,
			)
			.expect("a fresh load installs");

		assert!(slot.installed().is_some(), "the engine is resident");
		assert_eq!(*status.lock().unwrap(), EngineStatus::ready(Some(spec.id)));
		// event order: loading first (the wrapper emits the final status)
		assert_eq!(
			events.snapshot(),
			vec![format!("stt:Loading:Some({:?})", spec.id)]
		);
	}

	#[test]
	fn a_load_completing_after_a_newer_save_installs_publishes_and_rolls_back_nothing() {
		let (state, _dir) = temp_state("swap-stale");
		let spec = stt_spec("whisper-small-en");
		std::fs::write(state.model_path(&spec), b"stub").expect("model file");
		let slot = Arc::new(FakeSlot::empty());
		let status = Arc::new(Mutex::new(EngineStatus::missing()));
		let events = Arc::new(RecordingEvents::new());
		let rollbacks = Arc::new(Mutex::new(Vec::<String>::new()));
		let factory_called = Arc::new(AtomicBool::new(false));

		// the settings generation moves while the old plan's engine
		// load is in flight (the barrier inside the factory)
		let current_generation = Arc::new(AtomicU64::new(1));
		let captured_generation = 1u64;
		let (loaded_tx, loaded_rx) = mpsc::channel::<()>();
		let (release_tx, release_rx) = mpsc::channel::<()>();
		let release_rx = Arc::new(Mutex::new(release_rx));
		let state = Arc::new(state);
		let spec_id = spec.id;

		let worker = {
			let state = state.clone();
			let release_rx = release_rx.clone();
			let rollbacks = rollbacks.clone();
			let factory_called = factory_called.clone();
			let current_generation = current_generation.clone();
			let events = events.clone();
			let slot = slot.clone();
			let status = status.clone();
			std::thread::spawn(move || {
				state.swap_engine(
					&*events,
					&spec,
					&status,
					crate::notify_stt,
					&*slot,
					model_id_of,
					|| Ok(()),
					|_path| {
						factory_called.store(true, Ordering::SeqCst);
						loaded_tx.send(()).expect("test alive");
						// the multi-second engine load: the newer save
						// commits before this returns
						release_rx.lock().unwrap().recv().expect("released");
						Ok(FakeEngine {
							id: spec.id.to_string(),
						})
					},
					|prev| {
						rollbacks.lock().unwrap().push(prev.id.to_string());
						Ok(())
					},
					true,
					false,
					|| current_generation.load(Ordering::SeqCst) <= captured_generation,
				)
			})
		};

		loaded_rx.recv().expect("engine construction started");
		// the user saves newer settings while the old load runs
		current_generation.store(2, Ordering::SeqCst);
		release_tx.send(()).expect("release the old load");

		let result = worker.join().expect("swap thread");
		result.expect_err("a load that finished after a newer save must not activate");
		assert!(factory_called.load(Ordering::SeqCst));
		assert!(
			slot.installed().is_none(),
			"a stale load installs nothing: the newer loader owns the outcome"
		);
		assert!(
			rollbacks.lock().unwrap().is_empty(),
			"a stale load must not touch rollback"
		);
		// the only publication is the loading event from before the
		// save; nothing is published after it
		assert_eq!(
			events.snapshot(),
			vec![format!("stt:Loading:Some({:?})", spec_id)]
		);
		assert_eq!(*status.lock().unwrap(), EngineStatus::loading(spec_id));
	}

	#[test]
	fn an_already_resident_engine_is_not_reloaded() {
		let (state, _dir) = temp_state("swap-resident");
		let spec = stt_spec("whisper-small-en");
		let resident = Arc::new(FakeEngine {
			id: spec.id.to_string(),
		});
		let slot = FakeSlot::empty();
		slot.install(resident.clone());
		let status = Mutex::new(EngineStatus::ready(Some(spec.id)));
		let events = RecordingEvents::new();
		let factory_called = AtomicBool::new(false);

		let result = state.swap_engine(
			&events,
			&spec,
			&status,
			crate::notify_stt,
			&slot,
			model_id_of,
			|| Ok(()),
			|_path| {
				factory_called.store(true, Ordering::SeqCst);
				Ok(FakeEngine {
					id: spec.id.to_string(),
				})
			},
			|_prev| panic!("no load happens: no rollback"),
			true,
			false,
			|| true,
		);

		result.expect("the wanted engine is already there");
		assert!(
			!factory_called.load(Ordering::SeqCst),
			"an already-correct resident engine must not be reloaded"
		);
		assert!(
			Arc::ptr_eq(&slot.installed().expect("engine kept"), &resident),
			"the very same engine instance stays resident"
		);
		assert_eq!(*status.lock().unwrap(), EngineStatus::ready(Some(spec.id)));
		assert!(events.snapshot().is_empty(), "{:?}", events.snapshot());
	}

	#[test]
	fn a_failed_switch_rolls_back_and_reports_the_previous_model() {
		let (state, _dir) = temp_state("swap-rollback");
		let spec = stt_spec("whisper-small-en");
		let prev = stt_spec("whisper-tiny-en");
		let slot = FakeSlot::with(FakeEngine {
			id: prev.id.to_string(),
		});
		let status = Mutex::new(EngineStatus::missing());
		let events = RecordingEvents::new();
		let rollbacks: Mutex<Vec<String>> = Mutex::new(Vec::new());

		let result = state.swap_engine(
			&events,
			&spec,
			&status,
			crate::notify_stt,
			&slot,
			model_id_of,
			|| Ok(()),
			|_path| Err("corrupt file".into()),
			|restore| {
				rollbacks.lock().unwrap().push(restore.id.to_string());
				slot.install(Arc::new(FakeEngine {
					id: restore.id.to_string(),
				}));
				Ok(())
			},
			true,
			false,
			|| true,
		);

		let err = result.expect_err("the switch failed");
		assert!(
			err.contains("could not load whisper-small-en - whisper-tiny-en is still active"),
			"{err}"
		);
		assert_eq!(*rollbacks.lock().unwrap(), vec![prev.id.to_string()]);
		let after = status.lock().unwrap().clone();
		assert_eq!(after.state, EngineState::Ready);
		assert_eq!(after.model_id.as_deref(), Some(prev.id));
		assert!(
			after
				.error
				.unwrap()
				.contains("whisper-tiny-en is still active"),
			"the status says the previous model is active again"
		);
	}

	#[test]
	fn a_failed_first_load_without_rollback_publishes_error() {
		let (state, _dir) = temp_state("swap-error");
		let spec = stt_spec("whisper-small-en");
		let slot = FakeSlot::empty();
		let status = Mutex::new(EngineStatus::missing());
		let events = RecordingEvents::new();

		let result = state.swap_engine(
			&events,
			&spec,
			&status,
			crate::notify_stt,
			&slot,
			model_id_of,
			|| Ok(()),
			|_path| Err("corrupt file".into()),
			|_prev| panic!("nothing was loaded before: no rollback candidate"),
			true,
			false,
			|| true,
		);

		let err = result.expect_err("the load failed");
		assert_eq!(err, "model failed to load");
		assert!(slot.installed().is_none());
		assert_eq!(
			*status.lock().unwrap(),
			EngineStatus::error(Some(spec.id), "model failed to load")
		);
	}
}

#[cfg(test)]
mod load_path_tests {
	use super::{AppState, EngineStatus};
	use crate::StatusEvents;
	use std::sync::Mutex;

	struct RecordingEvents(Mutex<Vec<String>>);

	impl RecordingEvents {
		fn new() -> Self {
			Self(Mutex::new(Vec::new()))
		}
	}

	impl StatusEvents for RecordingEvents {
		fn llm_status(&self, status: &super::EngineStatus) {
			self.0
				.lock()
				.unwrap()
				.push(format!("llm:{:?}:{:?}", status.state, status.model_id));
		}
		fn stt_status(&self, status: &super::EngineStatus) {
			self.0
				.lock()
				.unwrap()
				.push(format!("stt:{:?}:{:?}", status.state, status.model_id));
		}
	}

	fn temp_state(name: &str) -> (AppState, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::create_dir_all(dir.path().join("models")).expect("models dir");
		let db = crate::db::Db::open(&dir.path().join(format!("{name}.db"))).expect("db");
		(AppState::new(db, dir.path().to_path_buf()), dir)
	}

	fn stt_spec() -> super::super::catalog::ModelSpec {
		super::super::catalog::find_model("whisper-small-en", super::super::catalog::ModelKind::Stt)
			.expect("catalog model")
			.clone()
	}

	#[test]
	fn a_stale_load_request_is_refused_before_any_status_change() {
		let (state, _dir) = temp_state("load-stale");
		let spec = stt_spec();
		// the load was decided at generation 0; a save has since bumped
		// the generation to 1
		state
			.mutate_ai_settings(|s| {
				s.stt_language = "fr-FR".into();
				Ok(())
			})
			.expect("newer save commits");
		let events = RecordingEvents::new();

		let result = state.load_stt_events(&events, &spec, 0);

		let err = result.expect_err("a stale load request must be refused");
		assert!(err.contains("settings changed"), "{err}");
		// refused before any status change: the status mutex still says
		// missing, and the only event (the wrapper's unconditional
		// final emit, unchanged from before) re-states that same
		// missing status - no loading blip ever published
		assert_eq!(*state.stt_status.lock().unwrap(), EngineStatus::missing());
		assert_eq!(
			events.0.lock().unwrap().as_slice(),
			["stt:Missing:None".to_string()].as_slice(),
			"no loading/ready/error may be published for a stale load"
		);
		// ...and the slot claim was released on the way out
		assert!(!state.stt_loading.load(std::sync::atomic::Ordering::SeqCst));
	}

	#[test]
	fn a_busy_slot_refuses_a_load_without_touching_status() {
		let (state, _dir) = temp_state("load-busy");
		let spec = stt_spec();
		state
			.stt_loading
			.store(true, std::sync::atomic::Ordering::SeqCst);
		let events = RecordingEvents::new();

		let result = state.load_stt_events(&events, &spec, 0);

		let err = result.expect_err("a busy slot must refuse the load");
		assert!(err.contains("already loading"), "{err}");
		assert_eq!(*state.stt_status.lock().unwrap(), EngineStatus::missing());
		assert!(events.0.lock().unwrap().is_empty());
		// the running load keeps its slot
		assert!(state.stt_loading.load(std::sync::atomic::Ordering::SeqCst));
	}
}

#[cfg(test)]
mod delete_flow_tests {
	use super::{AppState, EngineSlot, EngineState, EngineStatus};
	use crate::StatusEvents;
	use std::sync::atomic::Ordering;
	use std::sync::{mpsc, Arc, Mutex};
	use std::time::{Duration, Instant};

	/// A stand-in engine: nothing whisper/llama-shaped, just the id the
	/// delete flow reads.
	struct FakeEngine {
		id: String,
	}

	/// A stand-in engine slot holding a resident fake engine.
	struct FakeSlot(Mutex<Option<Arc<FakeEngine>>>);

	impl FakeSlot {
		fn with(id: &str) -> Self {
			Self(Mutex::new(Some(Arc::new(FakeEngine {
				id: id.to_string(),
			}))))
		}
	}

	impl EngineSlot<FakeEngine> for FakeSlot {
		fn installed(&self) -> Option<Arc<FakeEngine>> {
			self.0.lock().unwrap().clone()
		}
		fn vacate(&self) {
			*self.0.lock().unwrap() = None;
		}
		fn install(&self, engine: Arc<FakeEngine>) {
			*self.0.lock().unwrap() = Some(engine);
		}
	}

	struct RecordingEvents(Mutex<Vec<String>>);

	impl RecordingEvents {
		fn new() -> Self {
			Self(Mutex::new(Vec::new()))
		}
		fn snapshot(&self) -> Vec<String> {
			self.0.lock().unwrap().clone()
		}
	}

	impl StatusEvents for RecordingEvents {
		fn llm_status(&self, status: &EngineStatus) {
			self.0
				.lock()
				.unwrap()
				.push(format!("llm:{:?}:{:?}", status.state, status.model_id));
		}
		fn stt_status(&self, status: &EngineStatus) {
			self.0
				.lock()
				.unwrap()
				.push(format!("stt:{:?}:{:?}", status.state, status.model_id));
		}
	}

	fn temp_state(name: &str) -> (AppState, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::create_dir_all(dir.path().join("models")).expect("models dir");
		let db = crate::db::Db::open(&dir.path().join(format!("{name}.db"))).expect("db");
		(AppState::new(db, dir.path().to_path_buf()), dir)
	}

	fn stt_spec() -> super::super::catalog::ModelSpec {
		super::super::catalog::find_model("whisper-small-en", super::super::catalog::ModelKind::Stt)
			.expect("catalog model")
			.clone()
	}

	/// The production download guard: the delete runs UNDER the engine
	/// slot claim, whose flag a loading check would always read as
	/// busy - so the guard covers download bookkeeping only (load
	/// exclusivity is the claim's job).
	fn download_guard(
		state: &AppState,
		spec: &super::super::catalog::ModelSpec,
	) -> Option<&'static str> {
		if state
			.download_progress
			.lock()
			.unwrap()
			.contains_key(spec.id)
		{
			return Some("model is currently downloading");
		}
		if state.download_cancels.lock().unwrap().contains_key(spec.id) {
			return Some("model is still being set up - try again in a moment");
		}
		None
	}

	/// `delete_model_exclusive` for the stt kind against stand-ins.
	#[allow(clippy::too_many_arguments)]
	fn delete_stt(
		state: &AppState,
		spec: &super::super::catalog::ModelSpec,
		slot: &FakeSlot,
		status: &Mutex<EngineStatus>,
		events: &RecordingEvents,
		delete_files: &dyn Fn() -> Result<bool, String>,
		slot_wait: Duration,
	) -> Result<(), String> {
		let blocked = || download_guard(state, spec);
		state.delete_model_exclusive(
			events,
			spec,
			status,
			crate::notify_stt,
			slot,
			|engine| engine.id.as_str(),
			&blocked,
			delete_files,
			slot_wait,
		)
	}

	/// A delete_files closure that just removes the app copy.
	fn remove_app_copy<'a>(
		state: &'a AppState,
		spec: &'a super::super::catalog::ModelSpec,
	) -> Box<dyn Fn() -> Result<bool, String> + 'a> {
		Box::new(move || {
			let app_path = state.model_path(spec);
			if app_path.is_file() {
				std::fs::remove_file(&app_path).map_err(|e| e.to_string())?;
				return Ok(true);
			}
			Ok(false)
		})
	}

	/// F13: an activation/load starting at the delete boundary must be
	/// refused: the delete holds the engine slot claim for its whole
	/// operation, and loads take the same claim.
	#[test]
	fn a_load_starting_at_the_delete_boundary_is_refused_under_the_claim() {
		let (state, _dir) = temp_state("del-boundary");
		let spec = stt_spec();
		let app_path = state.model_path(&spec);
		std::fs::write(&app_path, b"stub").expect("model file");
		let slot = Arc::new(FakeSlot::with(spec.id));
		let status = Arc::new(Mutex::new(EngineStatus::ready(Some(spec.id))));
		let events = Arc::new(RecordingEvents::new());
		let state = Arc::new(state);

		// the delete signals once it is mid-deletion (past every guard,
		// inside its file removal) and waits for the release
		let (started_tx, started_rx) = mpsc::channel::<()>();
		let (release_tx, release_rx) = mpsc::channel::<()>();
		let release_rx = Arc::new(Mutex::new(release_rx));
		let deleter = {
			let state = state.clone();
			let spec = spec.clone();
			let slot = slot.clone();
			let status = status.clone();
			let events = events.clone();
			let app_path = app_path.clone();
			std::thread::spawn(move || {
				let delete_files = move || {
					started_tx.send(()).expect("test alive");
					release_rx.lock().unwrap().recv().expect("released");
					std::fs::remove_file(&app_path)
						.map_err(|e| e.to_string())
						.map(|()| true)
				};
				delete_stt(
					&state,
					&spec,
					&slot,
					&status,
					&events,
					&delete_files,
					Duration::from_secs(5),
				)
			})
		};

		started_rx
			.recv()
			.expect("the delete is mid-flight, holding the claim");
		// the load/activation entry the command layer uses, starting
		// exactly at the delete boundary
		let load_events = RecordingEvents::new();
		let generation = state.ai_settings_generation.load(Ordering::SeqCst);
		let err = state
			.load_stt_events(&load_events, &spec, generation)
			.expect_err("the load returns");
		assert!(
			err.contains("already loading"),
			"the load interleaved with the delete: {err}"
		);
		assert_eq!(
			*state.stt_status.lock().unwrap(),
			EngineStatus::missing(),
			"a refused load must not publish a loading status"
		);

		release_tx.send(()).expect("release the delete");
		deleter.join().unwrap().expect("the delete completes");

		assert!(!app_path.is_file(), "the model file is gone");
		assert!(slot.installed().is_none(), "the engine was unloaded");
		assert_eq!(*status.lock().unwrap(), EngineStatus::missing());
		assert_eq!(events.snapshot(), vec!["stt:Missing:None".to_string()]);
		assert!(
			!state.stt_loading.load(Ordering::SeqCst),
			"the claim is released"
		);
	}

	/// F13: a filesystem failure after the unload must still publish a
	/// truthful non-ready status before the Err returns - never leave
	/// ready behind for an engine the delete just unloaded.
	#[test]
	fn a_remove_file_failure_after_the_unload_publishes_non_ready_status_and_errs() {
		let (state, _dir) = temp_state("del-fserror");
		let spec = stt_spec();
		std::fs::write(state.model_path(&spec), b"stub").expect("model file");
		let slot = FakeSlot::with(spec.id);
		let status = Mutex::new(EngineStatus::ready(Some(spec.id)));
		let events = RecordingEvents::new();

		let err = delete_stt(
			&state,
			&spec,
			&slot,
			&status,
			&events,
			&|| Err("injected remove_file failure".to_string()),
			Duration::from_secs(5),
		)
		.expect_err("the filesystem failure surfaces");
		assert!(err.contains("injected remove_file failure"), "{err}");
		assert!(slot.installed().is_none(), "the engine was unloaded first");
		let after = status.lock().unwrap().clone();
		assert_ne!(
			after.state,
			EngineState::Ready,
			"status must never say ready for an engine the delete just unloaded"
		);
		assert_eq!(after.state, EngineState::Error);
		assert_eq!(after.model_id.as_deref(), Some(spec.id));
		assert!(
			after
				.error
				.as_deref()
				.unwrap()
				.contains("injected remove_file failure"),
			"{:?}",
			after.error
		);
		assert_eq!(
			events.snapshot(),
			vec![format!("stt:Error:Some({:?})", spec.id)]
		);
		assert!(
			!state.stt_loading.load(Ordering::SeqCst),
			"the claim is released"
		);
	}

	/// The delete waits out a running load of its kind instead of
	/// bulldozing past it (task-12 acquire semantics).
	#[test]
	fn a_delete_waits_out_a_running_load_of_its_kind_then_deletes() {
		let (state, _dir) = temp_state("del-waits");
		let spec = stt_spec();
		let app_path = state.model_path(&spec);
		std::fs::write(&app_path, b"stub").expect("model file");
		let slot = Arc::new(FakeSlot::with(spec.id));
		let status = Arc::new(Mutex::new(EngineStatus::ready(Some(spec.id))));
		let events = Arc::new(RecordingEvents::new());
		let state = Arc::new(state);
		// a load of this kind is running
		state.stt_loading.store(true, Ordering::SeqCst);
		// when the file removal actually ran, and when the load finished
		let deleted_at = Arc::new(Mutex::new(None::<Instant>));
		let load_finished_at = Arc::new(Mutex::new(None::<Instant>));

		let deleter = {
			let state = state.clone();
			let spec = spec.clone();
			let slot = slot.clone();
			let status = status.clone();
			let events = events.clone();
			let deleted_at = deleted_at.clone();
			std::thread::spawn(move || {
				let deleted_at = deleted_at.clone();
				let delete_files = || {
					*deleted_at.lock().unwrap() = Some(Instant::now());
					let app_path = state.model_path(&spec);
					if app_path.is_file() {
						std::fs::remove_file(&app_path).map_err(|e| e.to_string())?;
						return Ok(true);
					}
					Ok(false)
				};
				delete_stt(
					&state,
					&spec,
					&slot,
					&status,
					&events,
					&delete_files,
					Duration::from_secs(5),
				)
			})
		};
		std::thread::sleep(Duration::from_millis(150));
		// the load finishes; only now may the delete proceed
		*load_finished_at.lock().unwrap() = Some(Instant::now());
		state.stt_loading.store(false, Ordering::SeqCst);

		deleter
			.join()
			.unwrap()
			.expect("the delete completes after the load");
		assert!(
			deleted_at.lock().unwrap().unwrap() >= load_finished_at.lock().unwrap().unwrap(),
			"the delete must not touch files while a load of this kind runs"
		);
		assert!(!app_path.is_file(), "the model file is gone");
		assert!(slot.installed().is_none(), "the engine was unloaded");
		assert_eq!(*status.lock().unwrap(), EngineStatus::missing());
	}

	/// F13: the download guard is re-checked UNDER the claim, so a
	/// download that registers while the delete waits out a load is
	/// still caught.
	#[test]
	fn a_download_registering_while_the_delete_waits_is_caught_under_the_claim() {
		let (state, _dir) = temp_state("del-late-download");
		let spec = stt_spec();
		let app_path = state.model_path(&spec);
		std::fs::write(&app_path, b"stub").expect("model file");
		let slot = Arc::new(FakeSlot::with(spec.id));
		let status = Arc::new(Mutex::new(EngineStatus::ready(Some(spec.id))));
		let events = Arc::new(RecordingEvents::new());
		let state = Arc::new(state);
		// a load of this kind is running while the download starts
		state.stt_loading.store(true, Ordering::SeqCst);

		let deleter = {
			let state = state.clone();
			let spec = spec.clone();
			let slot = slot.clone();
			let status = status.clone();
			let events = events.clone();
			std::thread::spawn(move || {
				let delete_files = remove_app_copy(&state, &spec);
				delete_stt(
					&state,
					&spec,
					&slot,
					&status,
					&events,
					&delete_files,
					Duration::from_secs(5),
				)
			})
		};
		std::thread::sleep(Duration::from_millis(100));
		// the user starts a (re-)download while the delete is waiting
		state
			.download_progress
			.lock()
			.unwrap()
			.insert(spec.id.to_string(), 5.0);
		std::thread::sleep(Duration::from_millis(100));
		// the load finishes; the delete acquires the claim and must
		// still refuse: the download is in progress
		state.stt_loading.store(false, Ordering::SeqCst);

		let err = deleter
			.join()
			.unwrap()
			.expect_err("the download registered before the delete mutated");
		assert!(err.contains("downloading"), "{err}");
		assert!(app_path.is_file(), "a refused delete removes nothing");
		assert!(
			slot.installed().is_some(),
			"a refused delete unloads nothing"
		);
		assert_eq!(
			*status.lock().unwrap(),
			EngineStatus::ready(Some(spec.id)),
			"a refused delete publishes nothing"
		);
		assert!(events.snapshot().is_empty(), "{:?}", events.snapshot());
	}

	/// A wedged engine slot times the delete out with a busy error and
	/// no mutation: the holder keeps its slot (task-12 semantics).
	#[test]
	fn a_wedged_engine_slot_times_out_busy_without_mutating() {
		let (state, _dir) = temp_state("del-wedged");
		let spec = stt_spec();
		let app_path = state.model_path(&spec);
		std::fs::write(&app_path, b"stub").expect("model file");
		let slot = FakeSlot::with(spec.id);
		let status = Mutex::new(EngineStatus::ready(Some(spec.id)));
		let events = RecordingEvents::new();
		// a load of this kind never finishes
		state.stt_loading.store(true, Ordering::SeqCst);

		let err = delete_stt(
			&state,
			&spec,
			&slot,
			&status,
			&events,
			&remove_app_copy(&state, &spec),
			Duration::from_millis(200),
		)
		.expect_err("a wedged slot must refuse the delete");

		assert!(err.contains("loading"), "{err}");
		assert!(app_path.is_file(), "a refused delete removes nothing");
		assert!(
			slot.installed().is_some(),
			"a refused delete unloads nothing"
		);
		assert_eq!(*status.lock().unwrap(), EngineStatus::ready(Some(spec.id)));
		assert!(
			state.stt_loading.load(Ordering::SeqCst),
			"the wedged holder keeps its slot"
		);
		assert!(events.snapshot().is_empty(), "{:?}", events.snapshot());
	}

	/// A delete during an active download returns busy and touches
	/// nothing.
	#[test]
	fn a_delete_during_an_active_download_returns_busy() {
		let (state, _dir) = temp_state("del-download");
		let spec = stt_spec();
		let app_path = state.model_path(&spec);
		std::fs::write(&app_path, b"stub").expect("model file");
		let slot = FakeSlot::with(spec.id);
		let status = Mutex::new(EngineStatus::ready(Some(spec.id)));
		let events = RecordingEvents::new();
		state
			.download_progress
			.lock()
			.unwrap()
			.insert(spec.id.to_string(), 5.0);

		let err = delete_stt(
			&state,
			&spec,
			&slot,
			&status,
			&events,
			&remove_app_copy(&state, &spec),
			Duration::from_secs(5),
		)
		.expect_err("a download in progress must refuse the delete");

		assert!(err.contains("downloading"), "{err}");
		assert!(app_path.is_file());
		assert!(slot.installed().is_some());
		assert_eq!(*status.lock().unwrap(), EngineStatus::ready(Some(spec.id)));
	}

	/// The files may already be gone (removed by hand): the resident
	/// engine is still unloaded, and the delete reports missing - it
	/// must never leave a ready status behind an empty slot.
	#[test]
	fn a_missing_model_file_still_unloads_and_publishes_missing() {
		let (state, _dir) = temp_state("del-gone");
		let spec = stt_spec();
		let slot = FakeSlot::with(spec.id);
		let status = Mutex::new(EngineStatus::ready(Some(spec.id)));
		let events = RecordingEvents::new();

		let err = delete_stt(
			&state,
			&spec,
			&slot,
			&status,
			&events,
			&remove_app_copy(&state, &spec),
			Duration::from_secs(5),
		)
		.expect_err("nothing on disk belongs to the model");

		assert_eq!(err, "model file not found");
		assert!(slot.installed().is_none(), "the engine was unloaded");
		assert_eq!(*status.lock().unwrap(), EngineStatus::missing());
		assert_eq!(events.snapshot(), vec!["stt:Missing:None".to_string()]);
	}

	/// Task-10 semantics through the delete flow: the snapshot entry
	/// goes, a blob another revision still uses is retained and the
	/// delete still succeeds and reports missing truthfully.
	#[test]
	#[cfg(target_family = "unix")]
	fn a_partial_cache_removal_reports_the_retained_blob_truthfully() {
		let (state, _dir) = temp_state("del-partial");
		let spec = stt_spec();
		let app_path = state.model_path(&spec);
		std::fs::write(&app_path, b"app copy").expect("app copy");
		let slot = FakeSlot::with(spec.id);
		let status = Mutex::new(EngineStatus::ready(Some(spec.id)));
		let events = RecordingEvents::new();

		// a hub cache entry: our pinned snapshot link plus a foreign
		// revision holding a same-named file with different content
		let cache = tempfile::tempdir().expect("temp cache dir");
		let blob = crate::models::hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).expect("blobs dir");
		std::fs::write(&blob, b"shared blob bytes").expect("blob");
		crate::models::materialize_snapshot(cache.path(), &spec).expect("snapshot");
		let snapshots = blob.parent().unwrap().parent().unwrap().join("snapshots");
		let foreign = snapshots.join("foreign-rev");
		std::fs::create_dir_all(&foreign).expect("foreign rev");
		std::fs::write(foreign.join(spec.filename), b"someone else's revision")
			.expect("foreign file");

		let cache_path = cache.path().to_path_buf();
		let delete_files = || {
			let mut deleted = false;
			if app_path.is_file() {
				std::fs::remove_file(&app_path).map_err(|e| e.to_string())?;
				deleted = true;
			}
			deleted |= crate::models::remove_cached_model(&cache_path, &spec)?;
			Ok(deleted)
		};
		delete_stt(
			&state,
			&spec,
			&slot,
			&status,
			&events,
			&delete_files,
			Duration::from_secs(5),
		)
		.expect("the partial cache removal is a successful delete");

		assert!(!app_path.is_file(), "the app copy is gone");
		assert!(
			!snapshots.join(spec.sha256).join(spec.filename).exists(),
			"the pinned snapshot entry is gone"
		);
		assert!(
			foreign.join(spec.filename).is_file(),
			"a foreign revision's same-named file is retained"
		);
		assert!(
			blob.is_file(),
			"the blob stays while a foreign revision might use it"
		);
		assert!(slot.installed().is_none(), "the engine was unloaded");
		assert_eq!(*status.lock().unwrap(), EngineStatus::missing());
	}

	/// A successful repeat delete stays clean: busy-free "not found",
	/// no republication, no resurrected engine.
	#[test]
	fn a_successful_repeat_delete_stays_clean() {
		let (state, _dir) = temp_state("del-repeat");
		let spec = stt_spec();
		let app_path = state.model_path(&spec);
		std::fs::write(&app_path, b"stub").expect("model file");
		let slot = FakeSlot::with(spec.id);
		let status = Mutex::new(EngineStatus::ready(Some(spec.id)));
		let events = RecordingEvents::new();

		delete_stt(
			&state,
			&spec,
			&slot,
			&status,
			&events,
			&remove_app_copy(&state, &spec),
			Duration::from_secs(5),
		)
		.expect("the first delete succeeds");
		assert_eq!(*status.lock().unwrap(), EngineStatus::missing());
		assert_eq!(events.snapshot(), vec!["stt:Missing:None".to_string()]);

		let err = delete_stt(
			&state,
			&spec,
			&slot,
			&status,
			&events,
			&remove_app_copy(&state, &spec),
			Duration::from_secs(5),
		)
		.expect_err("nothing is left to delete");
		assert_eq!(err, "model file not found");
		assert!(slot.installed().is_none(), "no engine comes back");
		assert_eq!(
			*status.lock().unwrap(),
			EngineStatus::missing(),
			"a repeat delete republishes nothing"
		);
		assert_eq!(
			events.snapshot(),
			vec!["stt:Missing:None".to_string()],
			"a repeat delete publishes no second event"
		);
		assert!(
			!state.stt_loading.load(Ordering::SeqCst),
			"the claim is released"
		);
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
		let Some(claim) = EngineSlotClaim::acquire(&loading, Duration::from_secs(5)) else {
			panic!("the claim must succeed once the running load finishes");
		};
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
	fn a_wedged_slot_times_out_to_none_and_stays_owned_by_its_holder() {
		let loading = AtomicBool::new(true);
		let claim = EngineSlotClaim::acquire(&loading, Duration::from_millis(100));
		assert!(claim.is_none(), "a wedged slot must not authorize mutation");
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

	// ---- partial persistence failure (F06) ----
	//
	// Ordinary rows commit in one transaction BEFORE the sequential
	// secret writes, so a failing secret leaves the database holding
	// the new ordinary values while a stale cache still serves the old
	// ones. These regressions inject failures via SQLite triggers on
	// dev-only rows (debug builds keep secrets there), so they never
	// touch the developer's real keychain.

	/// Abort writes of one settings key behind Db's back (BEFORE INSERT
	/// covers the upsert's insert path for a row that does not exist
	/// yet; BEFORE DELETE covers a secret clear).
	fn fail_key_writes(path: &std::path::Path, key: &str, message: &str) {
		let conn = rusqlite::Connection::open(path).unwrap();
		conn.execute_batch(&format!(
			"CREATE TRIGGER fail_{key} BEFORE INSERT ON settings
			 WHEN NEW.key = '{key}'
			 BEGIN SELECT RAISE(ABORT, '{message}'); END;"
		))
		.unwrap();
	}

	fn fail_key_deletes(path: &std::path::Path, key: &str, message: &str) {
		let conn = rusqlite::Connection::open(path).unwrap();
		conn.execute_batch(&format!(
			"CREATE TRIGGER keep_{key} BEFORE DELETE ON settings
			 WHEN OLD.key = '{key}'
			 BEGIN SELECT RAISE(ABORT, '{message}'); END;"
		))
		.unwrap();
	}

	fn drop_fail_key_trigger(path: &std::path::Path, key: &str) {
		let conn = rusqlite::Connection::open(path).unwrap();
		conn.execute_batch(&format!("DROP TRIGGER fail_{key};"))
			.unwrap();
	}

	#[test]
	fn failed_secret_save_leaves_the_cache_on_the_committed_state() {
		// F06: the settings row commits, then the token write fails.
		// The cache must not keep serving the pre-save snapshot the
		// database no longer holds.
		let (state, dir) = temp_state("f06-truth");
		let db_path = dir.path().join("f06-truth.db");
		let _ = state.ai_settings(); // warm the cache
		fail_key_writes(
			&db_path,
			crate::keys::setting::dev_secret::HF_TOKEN,
			"injected secret write failure",
		);

		state
			.mutate_ai_settings(|s| {
				s.apply_updates(&serde_json::json!({
					"sttLanguage": "fr-FR",
					"extLlmModel": "changed-before-error",
					"hfToken": "fake-test-token",
				}))
			})
			.expect_err("the token write fails");

		// the cache reflects what the stores actually hold now
		assert_eq!(state.ai_settings().stt_language, "fr-FR");
		assert_eq!(state.ai_settings().ext_llm_model, "changed-before-error");
		assert_eq!(state.ai_settings().hf_token, "");
		// cold-restart equivalence: a fresh AppState loads the same state
		let cold = reopened(dir.path(), "f06-truth");
		assert_eq!(cold.ai_settings().stt_language, "fr-FR");
		assert_eq!(cold.ai_settings().ext_llm_model, "changed-before-error");
		assert_eq!(cold.ai_settings().hf_token, "");
	}

	#[test]
	fn secret_failures_name_the_failed_secret_and_keep_later_secrets_unwritten() {
		use crate::keys::setting::dev_secret;
		// (dev row to break, secret's error name) in the order save()
		// writes them; every round's patch touches all three secrets
		let cases = [
			(dev_secret::HF_TOKEN, "HuggingFace token"),
			(dev_secret::EXT_LLM_API_KEY, "external LLM API key"),
			(dev_secret::EXT_STT_API_KEY, "external STT API key"),
		];
		for (round, (row, name)) in cases.iter().enumerate() {
			let (state, dir) = temp_state(&format!("f06-name{round}"));
			let db_path = dir.path().join(format!("f06-name{round}.db"));
			let _ = state.ai_settings();
			fail_key_writes(&db_path, row, "injected secret write failure");

			let err = state
				.mutate_ai_settings(|s| {
					s.apply_updates(&serde_json::json!({
						"sttLanguage": "fr-FR",
						"hfToken": "fake-hf-token",
						"extLlmApiKey": "fake-llm-key",
						"extSttApiKey": "fake-stt-key",
					}))
				})
				.expect_err("one secret write fails");
			assert!(
				err.contains(&format!("storing the {name} failed")),
				"round {round}: the error must name the failed secret: {err}"
			);
			assert!(
				err.contains("injected secret write failure"),
				"round {round}: the store's own error must surface: {err}"
			);
			assert!(
				!err.contains("fake-hf-token")
					&& !err.contains("fake-llm-key")
					&& !err.contains("fake-stt-key"),
				"round {round}: no secret value in the error: {err}"
			);

			// the ordinary row committed; secrets written before the
			// failing one committed; the failing one and every later
			// one were not attempted
			let after = state.ai_settings();
			assert_eq!(after.stt_language, "fr-FR", "round {round}");
			let expected = [
				after.hf_token.as_str(),
				after.ext_llm_api_key.as_str(),
				after.ext_stt_api_key.as_str(),
			];
			for (index, secret_value) in expected.iter().enumerate() {
				let committed = index < round;
				let stored = if committed {
					vec!["fake-hf-token", "fake-llm-key", "fake-stt-key"][index]
				} else {
					""
				};
				assert_eq!(
					*secret_value, stored,
					"round {round}: secret {index} committed={committed}"
				);
			}
			// and the cache equals what a cold restart loads
			let cold = reopened(dir.path(), &format!("f06-name{round}"));
			assert_eq!(cold.ai_settings().stt_language, "fr-FR");
			assert_eq!(cold.ai_settings().hf_token, after.hf_token);
			assert_eq!(cold.ai_settings().ext_llm_api_key, after.ext_llm_api_key);
			assert_eq!(cold.ai_settings().ext_stt_api_key, after.ext_stt_api_key);
		}
	}

	#[test]
	fn clearing_failure_publishes_actual_stores_and_names_the_secret() {
		let (state, dir) = temp_state("f06-clear");
		let db_path = dir.path().join("f06-clear.db");
		state
			.mutate_ai_settings(|s| {
				s.apply_updates(&serde_json::json!({ "hfToken": "fake-old-token" }))
			})
			.expect("seed token");
		fail_key_deletes(
			&db_path,
			crate::keys::setting::dev_secret::HF_TOKEN,
			"database is locked",
		);

		let err = state
			.mutate_ai_settings(|s| {
				s.apply_updates(&serde_json::json!({
					"sttLanguage": "fr-FR",
					"hfToken": "",
				}))
			})
			.expect_err("the clear fails");
		assert!(
			err.contains("clearing the HuggingFace token failed"),
			"the error must name the failed clear: {err}"
		);

		// ordinary fields committed; the token survived in the store,
		// and the cache says so instead of claiming it was cleared
		assert_eq!(state.ai_settings().stt_language, "fr-FR");
		assert_eq!(state.ai_settings().hf_token, "fake-old-token");
		let cold = reopened(dir.path(), "f06-clear");
		assert_eq!(cold.ai_settings().stt_language, "fr-FR");
		assert_eq!(cold.ai_settings().hf_token, "fake-old-token");
	}

	#[test]
	fn settings_row_failure_keeps_cache_on_actual_stores_and_names_the_stage() {
		// The ordinary row transaction is atomic: nothing commits, so
		// the pre-save state IS the actual state - but the error must
		// still name the settings-row stage, not a secret.
		let (state, dir) = temp_state("f06-row");
		let db_path = dir.path().join("f06-row.db");
		let _ = state.ai_settings();
		fail_key_writes(
			&db_path,
			crate::keys::setting::AI_STT_LANGUAGE,
			"injected row write failure",
		);
		let gen_before = state.ai_settings_generation.load(Ordering::SeqCst);

		let err = state
			.mutate_ai_settings(|s| {
				s.apply_updates(&serde_json::json!({
					"sttLanguage": "fr-FR",
					"extLlmModel": "changed",
				}))
			})
			.expect_err("the settings row transaction fails");
		assert!(
			err.contains("could not save the settings"),
			"the error must name the failing stage: {err}"
		);
		assert!(!err.contains("token"), "no secret is involved: {err}");

		// nothing committed: cache and stores keep the previous values
		assert_eq!(state.ai_settings().stt_language, "en-US");
		assert_eq!(state.ai_settings().ext_llm_model, "");
		assert_eq!(
			reopened(dir.path(), "f06-row").ai_settings().stt_language,
			"en-US"
		);
		assert_eq!(
			state.ai_settings_generation.load(Ordering::SeqCst),
			gen_before,
			"a failed save does not bump the generation"
		);
	}

	#[test]
	fn unreadable_store_after_a_failed_save_drops_the_cache() {
		// The save fails AND the store cannot be read back: the cache
		// must be dropped (degraded mode) rather than left holding or
		// republishing unverified values.
		let (state, dir) = temp_state("f06-degraded");
		let db_path = dir.path().join("f06-degraded.db");
		let _ = state.ai_settings();
		{
			let conn = rusqlite::Connection::open(&db_path).unwrap();
			conn.execute_batch("DROP TABLE settings").unwrap();
		}

		let err = state
			.mutate_ai_settings(|s| s.apply_updates(&serde_json::json!({ "sttLanguage": "fr-FR" })))
			.expect_err("the save fails");
		assert!(
			err.contains("settings state unavailable after a failed save"),
			"degraded mode must be named: {err}"
		);
		assert!(
			state.ai_settings_cache.lock().unwrap().is_none(),
			"a store that cannot be read must force the next read to load cold"
		);
	}

	#[test]
	fn retry_after_partial_commit_preserves_committed_fields_and_retries_the_secret() {
		let (state, dir) = temp_state("f06-retry");
		let db_path = dir.path().join("f06-retry.db");
		let _ = state.ai_settings();
		fail_key_writes(
			&db_path,
			crate::keys::setting::dev_secret::HF_TOKEN,
			"injected secret write failure",
		);

		// the user saves; it fails on the token after the ordinary
		// fields committed
		state
			.mutate_ai_settings(|s| {
				s.apply_updates(&serde_json::json!({
					"sttLanguage": "fr-FR",
					"hfToken": "fake-test-token",
				}))
			})
			.expect_err("first attempt fails on the token");
		drop_fail_key_trigger(&db_path, crate::keys::setting::dev_secret::HF_TOKEN);

		// the retry goes through the boundary again: it must build on
		// the LATEST ACTUAL state, so the committed fr-FR survives and
		// the token (still absent in the store) is re-attempted
		state
			.mutate_ai_settings(|s| {
				s.apply_updates(&serde_json::json!({
					"extLlmModel": "new-model",
					"hfToken": "fake-test-token",
				}))
			})
			.expect("retry commits");

		let after = state.ai_settings();
		assert_eq!(
			after.stt_language, "fr-FR",
			"committed fields are not reverted"
		);
		assert_eq!(after.ext_llm_model, "new-model");
		assert_eq!(after.hf_token, "fake-test-token");
		let cold = reopened(dir.path(), "f06-retry");
		assert_eq!(cold.ai_settings().stt_language, "fr-FR");
		assert_eq!(cold.ai_settings().ext_llm_model, "new-model");
		assert_eq!(cold.ai_settings().hf_token, "fake-test-token");
	}
}
