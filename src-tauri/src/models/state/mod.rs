//! Runtime state: engines, statuses, load/swap/rollback, download
//! bookkeeping.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tauri::AppHandle;

use super::ai_settings::AiSettings;
use super::catalog::ModelSpec;
use super::download::{hf_cache_model_path, hf_hub_cache_candidates};
use crate::db::Db;
use crate::llm::LocalLlm;
use crate::stt::SttEngine;

/// A download's pause control: the token the running transfer polls,
/// plus the progress it stopped at. A `paused` entry survives its task
/// (until a resume, a cancel, a finished download or a delete) so the
/// Resume affordance can show where the transfer will continue.
#[derive(Clone)]
pub struct PauseEntry {
	pub token: Arc<AtomicBool>,
	pub pct: f64,
	pub paused: bool,
}

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
	/// Cancel token for the in-flight transcription (whisper, Apple
	/// Speech or the external endpoint). Its own token, for the same
	/// reason as the generation one.
	pub transcription_cancel: std::sync::Mutex<Arc<AtomicBool>>,
	/// model id -> progress percentage for in-flight downloads
	pub download_progress: std::sync::Mutex<std::collections::HashMap<String, f64>>,
	/// model id -> cancel token for in-flight downloads
	pub download_cancels: std::sync::Mutex<std::collections::HashMap<String, Arc<AtomicBool>>>,
	/// model id -> pause control for in-flight (or paused) downloads:
	/// `paused` entries outlive their transfer task so the UI can offer
	/// Resume from the recorded progress.
	pub download_pauses: std::sync::Mutex<std::collections::HashMap<String, PauseEntry>>,
	/// guards so only one load per engine kind runs at a time
	pub llm_loading: AtomicBool,
	pub stt_loading: AtomicBool,
	/// LLM generations started but not finished: a model switch waits
	/// for this to drain instead of vacating memory a live generation
	/// still maps, and the quick-capture shortcut refuses to navigate a
	/// generation into the void
	pub active_generations: std::sync::atomic::AtomicUsize,
	/// when the tray icon is hidden, closing the window quits the app
	/// (otherwise it would keep running with no way to reach it)
	pub quit_on_close: AtomicBool,
	/// shutdown began: new database writes refuse to start so the exit
	/// path can wait out the in-flight ones instead of cutting them off
	pub shutting_down: AtomicBool,
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

mod cache;
mod loading;

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
			transcription_cancel: std::sync::Mutex::new(Arc::new(AtomicBool::new(false))),
			download_progress: std::sync::Mutex::new(std::collections::HashMap::new()),
			download_cancels: std::sync::Mutex::new(std::collections::HashMap::new()),
			download_pauses: std::sync::Mutex::new(std::collections::HashMap::new()),
			llm_loading: AtomicBool::new(false),
			active_generations: std::sync::atomic::AtomicUsize::new(0),
			stt_loading: AtomicBool::new(false),
			quit_on_close: AtomicBool::new(false),
			shutting_down: AtomicBool::new(false),
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
}

#[cfg(test)]
mod delete_flow_tests;
#[cfg(test)]
mod load_path_tests;
#[cfg(test)]
mod rollback_tests;
#[cfg(test)]
mod settings_cache_tests;
#[cfg(test)]
mod slot_claim_tests;
#[cfg(test)]
mod swap_flow_tests;
