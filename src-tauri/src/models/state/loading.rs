//! Engine lifecycle: loading local models into the runtime slots,
//! swapping with rollback, and the exclusive delete flow.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use tauri::AppHandle;

use super::{
	lock, rollback_candidate, AppState, EngineSlot, EngineSlotClaim, EngineState, EngineStatus,
	LlmSlot, SttSlot, LLM_ROLLBACK_ON_SAME, STT_ROLLBACK_ON_SAME,
};
use crate::llm::LocalLlm;
use crate::models::{find_model, ModelKind, ModelSpec};
use crate::stt::SttEngine;

impl AppState {
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
			|engine| {
				// Already loaded = same model AND built with the window
				// the CURRENT settings resolve to: a ctx-only save must
				// rebuild the engine.
				engine.model_id.as_str() == spec.id
					&& engine.built_ctx()
						== crate::llm::effective_ctx_for(
							self.ai_settings().llm_ctx_tokens,
							engine.trained_ctx(),
						)
			},
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
				// The requested window comes from the CURRENT settings: a
				// save that landed while an older plan was loading still
				// builds the newest window (and the fresh() check retires
				// the stale outcome before it installs).
				LocalLlm::load(backend, path, spec.id, self.ai_settings().llm_ctx_tokens)
			},
			|prev| self.reload_llm(prev),
			LLM_ROLLBACK_ON_SAME,
			true,
			|| self.ai_settings_generation.load(Ordering::SeqCst) <= generation,
		)
	}

	#[allow(clippy::too_many_arguments)]
	pub(super) fn swap_engine<E>(
		&self,
		events: &dyn crate::StatusEvents,
		spec: &ModelSpec,
		status: &std::sync::Mutex<EngineStatus>,
		notify: fn(&EngineStatus, &dyn crate::StatusEvents),
		slot: &dyn EngineSlot<E>,
		model_id_of: fn(&E) -> &str,
		unchanged: impl Fn(&E) -> bool,
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
		// working engine (or blip its status through loading). The
		// predicate decides what "already wanted" means per engine kind
		// (see `unchanged` above) - for the LLM that includes the context
		// window, so a ctx-only save rebuilds.
		if slot
			.installed()
			.as_ref()
			.is_some_and(|engine| unchanged(engine))
		{
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
				// A hub-cache file with the right name but wrong bytes
				// fails with a cryptic decoder error: name it for what
				// it is so the fix (delete it or re-download) is obvious.
				let hub_sourced = self.resolve_model_file(spec) != Some(self.model_path(spec));
				let mismatched = hub_sourced
					&& std::fs::metadata(&path)
						.map(|m| m.len() != spec.size_bytes)
						.unwrap_or(false);
				let e = if mismatched {
					format!(
						"the cached copy in {} does not match the pinned model; delete it or re-download",
						path.display()
					)
				} else {
					e
				};
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
		// Restored under the CURRENT settings' window, so the rolled-back
		// engine matches what a retry of the same save would build.
		let engine = LocalLlm::load(backend, &path, spec.id, self.ai_settings().llm_ctx_tokens)?;
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
			// whisper has no ctx-shaped setting: same model id = done
			|engine| engine.model_id.as_str() == spec.id,
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
