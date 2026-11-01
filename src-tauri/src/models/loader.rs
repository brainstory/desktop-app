//! Model-load planning and reconciliation: what loads or unloads at
//! startup and after settings changes, and the status-event sink the
//! load flow publishes through. Split out of lib.rs with its tests.

use tauri::{AppHandle, Emitter, Manager};

use crate::models;
use crate::models::{find_model, AiSettings, AppState, EngineSlotClaim, ModelKind};

/// Delivery of engine-status events. Production emits Tauri events to
/// the webview; tests record or drop them, so the load/reconcile flow
/// is exercisable without an app handle.
pub(crate) trait StatusEvents {
	fn llm_status(&self, status: &super::EngineStatus);
	fn stt_status(&self, status: &super::EngineStatus);
	/// An owned emitter for live model-load progress (0..=1); None when
	/// the sink has no event channel (tests).
	fn llm_progress_emitter(&self) -> Option<std::sync::Arc<dyn Fn(f32) + Send + Sync>> {
		None
	}
}

/// Production event sink: the app's Tauri channels.
pub(crate) struct AppStatusEvents<'a>(pub &'a AppHandle);

impl StatusEvents for AppStatusEvents<'_> {
	fn llm_status(&self, status: &super::EngineStatus) {
		if let Err(e) = self.0.emit("llm-status", status) {
			log::warn!("failed to emit llm-status: {e}");
		}
	}
	fn llm_progress_emitter(&self) -> Option<std::sync::Arc<dyn Fn(f32) + Send + Sync>> {
		let app = self.0.clone();
		Some(std::sync::Arc::new(move |progress: f32| {
			// the loading status re-emitted with its progress field
			let status = super::EngineStatus {
				state: models::EngineState::Loading,
				model_id: None,
				error: None,
				progress: Some(progress),
			};
			if let Err(e) = app.emit("llm-status", &status) {
				log::warn!("failed to emit llm-status progress: {e}");
			}
		}))
	}
	fn stt_status(&self, status: &super::EngineStatus) {
		if let Err(e) = self.0.emit("stt-status", status) {
			log::warn!("failed to emit stt-status: {e}");
		}
	}
}

/// Status delivery for one engine kind, as a plain fn so the generic
/// swap flow can take it as a parameter.
pub(crate) fn notify_llm(status: &super::EngineStatus, events: &dyn StatusEvents) {
	events.llm_status(status);
}

pub(crate) fn notify_stt(status: &super::EngineStatus, events: &dyn StatusEvents) {
	events.stt_status(status);
}

/// Load active models at startup / after settings changes. Heavy loading
/// happens on a background thread so the UI starts instantly.
/// `generation` is the settings generation `settings` was captured at,
/// so this run can tell newer committed settings from its own snapshot.
pub(crate) fn spawn_model_loader(app: AppHandle, settings: AiSettings, generation: u64) {
	std::thread::spawn(move || run_model_loader(app, settings, generation));
}

/// Startup path: migrate legacy app-dir downloads into the hub cache
/// first (hash-verified, so a corrupt file never poisons a
/// content-addressed store), then load. Ordered so the loader never
/// mmaps a file the migration is about to move.
pub(crate) fn spawn_migration_and_model_loader(
	app: AppHandle,
	settings: AiSettings,
	generation: u64,
) {
	std::thread::spawn(move || {
		if let Some(state) = app.try_state::<AppState>() {
			super::migrate_legacy_models(&state.models_dir(), &super::primary_hub_cache());
		}
		run_model_loader(app, settings, generation);
	});
}

/// What the model loader does with the whisper slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WhisperPlan {
	/// load this (downloaded) catalog model
	Load(&'static str),
	/// unknown or not-downloaded model: unload whatever is resident and
	/// report "missing", so the runtime matches the reported status
	Missing,
	/// whisper is not needed at all: free its memory, status untouched
	Unload,
}

/// The STT status the loader reports after handling the whisper slot,
/// when something other than whisper's own load decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SttStatusPlan {
	/// the external endpoint transcribes
	External,
	/// Apple Speech transcribes
	AppleReady,
	/// explicit Apple on a system without it: whisper stands in, and the
	/// status says why
	AppleUnsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LlmPlan {
	External,
	Load(&'static str),
	Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LoadPlan {
	pub(crate) whisper: WhisperPlan,
	pub(crate) stt_status: Option<SttStatusPlan>,
	pub(crate) llm: LlmPlan,
}

/// The loader's whole decision, pure: which engines to load or unload
/// and which status to report, given the settings, whether Apple Speech
/// exists here, and which catalog models are downloaded.
pub(crate) fn plan_model_load(
	settings: &AiSettings,
	apple_available: bool,
	downloaded: impl Fn(&models::ModelSpec) -> bool,
) -> LoadPlan {
	use models::SpeechEngine;
	let whisper_model = match find_model(&settings.stt_model, ModelKind::Stt) {
		Some(spec) if downloaded(spec) => WhisperPlan::Load(spec.id),
		_ => WhisperPlan::Missing,
	};
	// STT: the external endpoint when switched on; otherwise the engine setting picks
	// Apple Speech (macOS 26+, zero downloads) or local whisper.
	let (whisper, stt_status) = if settings.uses_external_stt() {
		(WhisperPlan::Unload, Some(SttStatusPlan::External))
	} else if settings.stt_engine == SpeechEngine::Apple && !apple_available {
		(whisper_model, Some(SttStatusPlan::AppleUnsupported))
	} else {
		match settings.stt_engine.effective_with(apple_available) {
			// auto: keep a downloaded whisper hot as the fallback behind
			// the Apple engine; explicit apple: free whisper's memory
			SpeechEngine::Apple if settings.stt_engine == SpeechEngine::Auto => {
				(whisper_model, Some(SttStatusPlan::AppleReady))
			}
			SpeechEngine::Apple => (WhisperPlan::Unload, Some(SttStatusPlan::AppleReady)),
			// whisper (effective_with never reports Auto)
			_ => (whisper_model, None),
		}
	};
	// LLM: load the local model unless external mode is active.
	let llm = if settings.uses_external_llm() {
		LlmPlan::External
	} else {
		match find_model(&settings.llm_model, ModelKind::Llm) {
			Some(spec) if downloaded(spec) => LlmPlan::Load(spec.id),
			_ => LlmPlan::Missing,
		}
	};
	LoadPlan {
		whisper,
		stt_status,
		llm,
	}
}

fn run_model_loader(app: AppHandle, settings: AiSettings, generation: u64) {
	let Some(state) = app.try_state::<AppState>() else {
		return;
	};
	let events = AppStatusEvents(&app);
	let stt_load = |spec: &models::ModelSpec| state.load_stt_events(&events, spec, generation);
	let llm_load = |spec: &models::ModelSpec| state.load_llm_events(&events, spec, generation);
	reconcile_model_state(
		&state,
		&events,
		&settings,
		generation,
		SLOT_WAIT,
		crate::apple::speech_available(),
		&stt_load,
		&llm_load,
	);
}

/// Wait this long for an in-flight engine load before giving up on
/// reconciling that slot in this run.
const SLOT_WAIT: std::time::Duration = std::time::Duration::from_secs(300);

/// True when `generation` is obsolete: a newer committed settings save
/// exists, so work captured at `generation` must not mutate the runtime
/// or publish status - the newer run owns the outcome. Equal
/// generations mean the same settings: same plan, harmless.
fn settings_stale(state: &AppState, generation: u64) -> bool {
	state
		.ai_settings_generation
		.load(std::sync::atomic::Ordering::SeqCst)
		> generation
}

/// Bring the runtime in line with `settings` captured at `generation`:
/// the one publication path for both engine slots. Stale runs install
/// nothing and publish nothing; a slot that stays busy past `slot_wait`
/// aborts this run's arm for it rather than mutating unclaimed.
///
/// `stt_load`/`llm_load` perform the claimed engine loads (the test
/// seam; production uses [`AppState::load_stt_events`]/
/// [`AppState::load_llm_events`], which claim the slot themselves and
/// re-verify the generation under that claim, closing the interval
/// between this run releasing its claim and the load reacquiring it).
#[allow(clippy::too_many_arguments)]
fn reconcile_model_state(
	state: &AppState,
	events: &dyn StatusEvents,
	settings: &AiSettings,
	generation: u64,
	slot_wait: std::time::Duration,
	apple_available: bool,
	stt_load: &dyn Fn(&models::ModelSpec) -> Result<(), String>,
	llm_load: &dyn Fn(&models::ModelSpec) -> Result<(), String>,
) {
	// Stale before any work: a newer committed save exists and its own
	// loader owns the outcome.
	if settings_stale(state, generation) {
		return;
	}
	let plan = plan_model_load(settings, apple_available, |spec| {
		state.is_model_downloaded(spec)
	});
	// The catalog scan above is file-system work; still current?
	if settings_stale(state, generation) {
		return;
	}

	reconcile_whisper_slot(state, events, &plan, generation, slot_wait, stt_load);
	reconcile_llm_slot(state, events, &plan, generation, slot_wait, llm_load);
}

/// The published status for a plan decided by something other than
/// whisper's own load (the external endpoint or the Apple Speech
/// setting).
fn stt_status_for(plan: SttStatusPlan) -> super::EngineStatus {
	match plan {
		SttStatusPlan::External => super::EngineStatus::external(),
		SttStatusPlan::AppleReady => super::EngineStatus::ready(Some("apple-speech")),
		SttStatusPlan::AppleUnsupported => super::EngineStatus::error(
			Some("apple-speech"),
			"Apple Speech requires macOS 26+ - using whisper instead",
		),
	}
}

/// The whisper slot's share of one reconciliation run.
fn reconcile_whisper_slot(
	state: &AppState,
	events: &dyn StatusEvents,
	plan: &LoadPlan,
	generation: u64,
	slot_wait: std::time::Duration,
	stt_load: &dyn Fn(&models::ModelSpec) -> Result<(), String>,
) {
	match plan.whisper {
		WhisperPlan::Load(id) => {
			// Wait out any in-flight load of this slot, then hand the
			// slot to the load path itself: it re-claims and re-verifies
			// the generation under that claim, so nothing can slip
			// through the release-and-reacquire handoff below.
			match EngineSlotClaim::acquire(&state.stt_loading, slot_wait) {
				Some(claim) => {
					if !settings_stale(state, generation) {
						claim.release();
						let mut loaded = false;
						if let Some(spec) = find_model(id, ModelKind::Stt) {
							match stt_load(spec) {
								Ok(()) => loaded = true,
								Err(e) => log::error!("startup STT load failed: {e}"),
							}
						}
						// The load path leaves the slot reporting
						// whisper itself ("ready: whisper-..."). When
						// another engine actually transcribes (Apple
						// Speech standing in front of, or instead of,
						// whisper), that status is wrong: re-publish the
						// plan's status under a fresh claim, so a
						// settings save that landed mid-load still wins.
						if loaded {
							if let Some(status) = plan.stt_status {
								match EngineSlotClaim::acquire(&state.stt_loading, slot_wait) {
									Some(_claim) => {
										if !settings_stale(state, generation) {
											*state
												.stt_status
												.lock()
												.unwrap_or_else(|e| e.into_inner()) = stt_status_for(status);
											state.emit_stt_status_events(events);
										}
									}
									None => log::warn!(
										"stt slot still busy after {slot_wait:?}; keeping the whisper load's status"
									),
								}
							}
						}
					}
				}
				None => {
					log::warn!(
						"stt slot still busy after {slot_wait:?}; aborting this run's whisper reconciliation"
					);
				}
			}
		}
		WhisperPlan::Missing | WhisperPlan::Unload => {
			// Hold the slot while unloading: an unload racing a running
			// load would be undone when that load installs its engine,
			// contradicting the reported status. A wedged holder aborts
			// this arm instead of authorizing an unclaimed mutation.
			match EngineSlotClaim::acquire(&state.stt_loading, slot_wait) {
				Some(_claim) => {
					// Re-check under the claim: a newer run may have
					// committed while this one waited for the slot.
					if !settings_stale(state, generation) {
						state.runtime.lock().unwrap_or_else(|e| e.into_inner()).stt = None;
						if plan.whisper == WhisperPlan::Missing {
							*state.stt_status.lock().unwrap_or_else(|e| e.into_inner()) =
								super::EngineStatus::missing();
							state.emit_stt_status_events(events);
						}
						if let Some(status) = plan.stt_status {
							*state.stt_status.lock().unwrap_or_else(|e| e.into_inner()) =
								stt_status_for(status);
							state.emit_stt_status_events(events);
						}
					}
				}
				None => {
					log::warn!(
						"stt slot still busy after {slot_wait:?}; aborting this run's whisper reconciliation"
					);
				}
			}
		}
	}
}

/// The LLM slot's share of one reconciliation run.
fn reconcile_llm_slot(
	state: &AppState,
	events: &dyn StatusEvents,
	plan: &LoadPlan,
	generation: u64,
	slot_wait: std::time::Duration,
	llm_load: &dyn Fn(&models::ModelSpec) -> Result<(), String>,
) {
	match plan.llm {
		LlmPlan::Load(id) => {
			// Same handoff as the whisper slot above: wait out any
			// in-flight load, then the load path re-claims and
			// re-verifies under its own claim.
			match EngineSlotClaim::acquire(&state.llm_loading, slot_wait) {
				Some(claim) => {
					if !settings_stale(state, generation) {
						claim.release();
						if let Some(spec) = find_model(id, ModelKind::Llm) {
							if let Err(e) = llm_load(spec) {
								log::error!("startup LLM load failed: {e}");
							}
						}
					}
				}
				None => {
					log::warn!(
						"llm slot still busy after {slot_wait:?}; aborting this run's llm reconciliation"
					);
				}
			}
		}
		LlmPlan::External | LlmPlan::Missing => {
			match EngineSlotClaim::acquire(&state.llm_loading, slot_wait) {
				Some(_claim) => {
					if !settings_stale(state, generation) {
						state.runtime.lock().unwrap_or_else(|e| e.into_inner()).llm = None;
						*state.llm_status.lock().unwrap_or_else(|e| e.into_inner()) =
							if plan.llm == LlmPlan::External {
								super::EngineStatus::external()
							} else {
								super::EngineStatus::missing()
							};
						state.emit_llm_status_events(events);
					}
				}
				None => {
					log::warn!(
						"llm slot still busy after {slot_wait:?}; aborting this run's llm reconciliation"
					);
				}
			}
		}
	}
}

#[cfg(test)]
mod loader_plan_tests {
	use super::{plan_model_load, LlmPlan, LoadPlan, SttStatusPlan, WhisperPlan};
	use crate::models::{AiSettings, LlmMode, SpeechEngine, SttMode};

	fn settings(engine: SpeechEngine) -> (AiSettings, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		let db = crate::db::Db::open(&dir.path().join("t.db")).expect("db");
		let mut s = AiSettings::load(&db);
		s.stt_engine = engine;
		s.stt_model = "whisper-small-en".into();
		s.llm_model = "gemma-4-E4B".into();
		s.llm_mode = LlmMode::Local;
		s.stt_mode = SttMode::Local;
		s.ext_stt_base_url = String::new();
		(s, dir)
	}

	const ALL: fn(&crate::models::ModelSpec) -> bool = |_| true;
	const NONE: fn(&crate::models::ModelSpec) -> bool = |_| false;

	#[test]
	fn whisper_mode_loads_downloaded_models_and_reports_missing_ones() {
		let (s, _d) = settings(SpeechEngine::Whisper);
		assert_eq!(
			plan_model_load(&s, true, ALL),
			LoadPlan {
				whisper: WhisperPlan::Load("whisper-small-en"),
				stt_status: None,
				llm: LlmPlan::Load("gemma-4-E4B"),
			}
		);
		let plan = plan_model_load(&s, true, NONE);
		assert_eq!(plan.whisper, WhisperPlan::Missing);
		assert_eq!(plan.llm, LlmPlan::Missing);
		// an unknown (hand-edited) id is missing too, never a stale engine
		let (mut s, _d) = settings(SpeechEngine::Whisper);
		s.stt_model = "whisper-gone".into();
		s.llm_model = "llm-gone".into();
		let plan = plan_model_load(&s, true, ALL);
		assert_eq!(plan.whisper, WhisperPlan::Missing);
		assert_eq!(plan.llm, LlmPlan::Missing);
	}

	#[test]
	fn external_endpoints_unload_the_local_engines() {
		let (mut s, _d) = settings(SpeechEngine::Auto);
		s.ext_stt_base_url = "http://localhost:9000".into();
		s.stt_mode = SttMode::External;
		s.llm_mode = LlmMode::External;
		assert_eq!(
			plan_model_load(&s, true, ALL),
			LoadPlan {
				whisper: WhisperPlan::Unload,
				stt_status: Some(SttStatusPlan::External),
				llm: LlmPlan::External,
			}
		);
	}

	#[test]
	fn a_saved_stt_url_does_not_route_stt_externally_in_local_mode() {
		let (mut s, _d) = settings(SpeechEngine::Whisper);
		s.ext_stt_base_url = "http://localhost:9000".into();
		s.stt_mode = SttMode::Local;
		let plan = plan_model_load(&s, true, ALL);
		assert_eq!(plan.whisper, WhisperPlan::Load("whisper-small-en"));
		assert_eq!(plan.stt_status, None);
	}

	#[test]
	fn apple_speech_keeps_whisper_only_as_the_auto_fallback() {
		// auto + Apple available: Apple transcribes, whisper stays hot
		let (s, _d) = settings(SpeechEngine::Auto);
		let plan = plan_model_load(&s, true, ALL);
		assert_eq!(plan.whisper, WhisperPlan::Load("whisper-small-en"));
		assert_eq!(plan.stt_status, Some(SttStatusPlan::AppleReady));
		// explicit apple: whisper is freed, not loaded
		let (s, _d) = settings(SpeechEngine::Apple);
		let plan = plan_model_load(&s, true, ALL);
		assert_eq!(plan.whisper, WhisperPlan::Unload);
		assert_eq!(plan.stt_status, Some(SttStatusPlan::AppleReady));
		// auto without Apple Speech is plain whisper
		let (s, _d) = settings(SpeechEngine::Auto);
		let plan = plan_model_load(&s, false, ALL);
		assert_eq!(plan.whisper, WhisperPlan::Load("whisper-small-en"));
		assert_eq!(plan.stt_status, None);
	}

	#[test]
	fn explicit_apple_without_apple_speech_falls_back_and_says_why() {
		let (s, _d) = settings(SpeechEngine::Apple);
		let plan = plan_model_load(&s, false, ALL);
		assert_eq!(plan.whisper, WhisperPlan::Load("whisper-small-en"));
		assert_eq!(plan.stt_status, Some(SttStatusPlan::AppleUnsupported));
		let plan = plan_model_load(&s, false, NONE);
		assert_eq!(plan.whisper, WhisperPlan::Missing);
		assert_eq!(plan.stt_status, Some(SttStatusPlan::AppleUnsupported));
	}
}

#[cfg(test)]
mod loader_reconcile_tests {
	use super::{reconcile_model_state, StatusEvents};
	use crate::models::{self, AiSettings, AppState, LlmMode, ModelKind, SpeechEngine, SttMode};
	use std::sync::{Arc, Mutex};
	use std::time::Duration;

	/// The slot wait for tests: long enough to observe a real wait,
	/// short enough that a wedged slot aborts quickly.
	const SHORT_WAIT: Duration = Duration::from_millis(200);

	/// Records every published status event, tagged by engine kind.
	struct RecordingEvents(Mutex<Vec<String>>);

	impl RecordingEvents {
		fn new() -> Arc<Self> {
			Arc::new(Self(Mutex::new(Vec::new())))
		}
		fn snapshot(&self) -> Vec<String> {
			self.0.lock().unwrap().clone()
		}
	}

	impl StatusEvents for RecordingEvents {
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

	/// Stand-in for the claimed engine load: records each requested
	/// model and leaves statuses untouched, so tests observe exactly
	/// what the reconciliation itself publishes.
	#[derive(Default)]
	struct LoadRecorder {
		requests: Mutex<Vec<String>>,
	}

	impl LoadRecorder {
		fn record(&self, spec: &models::ModelSpec) -> Result<(), String> {
			self.requests.lock().unwrap().push(spec.id.to_string());
			Ok(())
		}
		fn requests(&self) -> Vec<String> {
			self.requests.lock().unwrap().clone()
		}
	}

	fn loader_state(name: &str) -> (Arc<AppState>, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::create_dir_all(dir.path().join("models")).expect("models dir");
		let db = crate::db::Db::open(&dir.path().join(format!("{name}.db"))).expect("db");
		(Arc::new(AppState::new(db, dir.path().to_path_buf())), dir)
	}

	/// A downloaded catalog model is a stub file in the models dir:
	/// `is_model_downloaded` only checks existence.
	fn mark_downloaded(state: &AppState, id: &str, kind: ModelKind) {
		let spec = crate::models::find_model(id, kind).expect("catalog model");
		std::fs::write(state.model_path(spec), b"stub").expect("write stub model file");
	}

	/// Local-routing settings naming the given models.
	fn local_settings(state: &AppState, stt: &str, llm: &str) -> AiSettings {
		let mut s = state.ai_settings();
		s.stt_engine = SpeechEngine::Whisper;
		s.stt_mode = SttMode::Local;
		s.stt_model = stt.into();
		s.llm_mode = LlmMode::Local;
		s.llm_model = llm.into();
		s.ext_stt_base_url = String::new();
		s
	}

	fn external_settings(state: &AppState) -> AiSettings {
		let mut s = state.ai_settings();
		s.stt_mode = SttMode::External;
		s.ext_stt_base_url = "http://localhost:9000".into();
		s.llm_mode = LlmMode::External;
		s.ext_llm_base_url = "http://localhost:9001".into();
		s
	}

	#[test]
	fn a_stale_run_loads_nothing_and_publishes_nothing() {
		let (state, _dir) = loader_state("stale-run");
		mark_downloaded(&state, "whisper-small-en", ModelKind::Stt);
		mark_downloaded(&state, "gemma-4-E4B", ModelKind::Llm);
		// this run was decided at generation 0 from local settings...
		let captured = local_settings(&state, "whisper-small-en", "gemma-4-E4B");
		// ...then the user saved external routing (generation 1) before
		// the run got any further
		state
			.mutate_ai_settings(|s| {
				s.stt_mode = SttMode::External;
				s.ext_stt_base_url = "http://localhost:9000".into();
				s.llm_mode = LlmMode::External;
				s.ext_llm_base_url = "http://localhost:9001".into();
				Ok(())
			})
			.expect("save external settings");

		let events = RecordingEvents::new();
		let stt_loads = Arc::new(LoadRecorder::default());
		let llm_loads = Arc::new(LoadRecorder::default());
		let stt_runner = |spec: &models::ModelSpec| stt_loads.record(spec);
		let llm_runner = |spec: &models::ModelSpec| llm_loads.record(spec);
		reconcile_model_state(
			&state,
			&*events,
			&captured,
			0,
			SHORT_WAIT,
			false,
			&stt_runner,
			&llm_runner,
		);

		assert!(
			stt_loads.requests().is_empty() && llm_loads.requests().is_empty(),
			"a stale run must start no engine loads (stt: {:?}, llm: {:?})",
			stt_loads.requests(),
			llm_loads.requests()
		);
		assert!(
			events.snapshot().is_empty(),
			"a stale run must publish no status: {:?}",
			events.snapshot()
		);
	}

	#[test]
	fn aba_model_changes_end_on_the_newest_choice_without_stale_work() {
		let (state, _dir) = loader_state("aba");
		mark_downloaded(&state, "whisper-small-en", ModelKind::Stt);
		mark_downloaded(&state, "whisper-tiny-en", ModelKind::Stt);
		let a = local_settings(&state, "whisper-small-en", "gemma-4-E4B");
		let b = local_settings(&state, "whisper-tiny-en", "gemma-4-E4B");

		// three saves commit in order A, B, A; the loaders only run
		// after the newest save, so the first two runs are stale
		let generations: Vec<u64> = ["whisper-small-en", "whisper-tiny-en", "whisper-small-en"]
			.iter()
			.map(|id| {
				state
					.mutate_ai_settings(|s| {
						s.stt_model = id.to_string();
						Ok(())
					})
					.expect("commit")
					.1
			})
			.collect();

		let events = RecordingEvents::new();
		let stt_loads = Arc::new(LoadRecorder::default());
		let llm_loads = Arc::new(LoadRecorder::default());
		for (captured, generation) in [
			(&a, generations[0]),
			(&b, generations[1]),
			(&a, generations[2]),
		] {
			let stt_runner = |spec: &models::ModelSpec| stt_loads.record(spec);
			let llm_runner = |spec: &models::ModelSpec| llm_loads.record(spec);
			reconcile_model_state(
				&state,
				&*events,
				captured,
				generation,
				SHORT_WAIT,
				false,
				&stt_runner,
				&llm_runner,
			);
		}

		assert_eq!(
			stt_loads.requests(),
			vec!["whisper-small-en".to_string()],
			"only the newest run may load; the stale A and B runs must not"
		);
		// gemma was never downloaded: the newest run plans Missing for
		// the LLM, so no LLM load starts either
		assert!(
			llm_loads.requests().is_empty(),
			"{:?}",
			llm_loads.requests()
		);
	}

	#[test]
	fn a_stalled_slot_timeout_aborts_instead_of_mutating_unclaimed() {
		let (state, _dir) = loader_state("stalled");
		let captured = external_settings(&state);
		// wedged in-flight loads hold both slots
		state
			.stt_loading
			.store(true, std::sync::atomic::Ordering::SeqCst);
		state
			.llm_loading
			.store(true, std::sync::atomic::Ordering::SeqCst);
		// the currently working engines are reported ready
		*state.stt_status.lock().unwrap() =
			crate::models::EngineStatus::ready(Some("whisper-small-en"));
		*state.llm_status.lock().unwrap() = crate::models::EngineStatus::ready(Some("gemma-4-E4B"));

		let events = RecordingEvents::new();
		let stt_loads = Arc::new(LoadRecorder::default());
		let llm_loads = Arc::new(LoadRecorder::default());
		let stt_runner = |spec: &models::ModelSpec| stt_loads.record(spec);
		let llm_runner = |spec: &models::ModelSpec| llm_loads.record(spec);
		reconcile_model_state(
			&state,
			&*events,
			&captured,
			0,
			SHORT_WAIT,
			false,
			&stt_runner,
			&llm_runner,
		);

		// a timed-out run must leave every observable exactly as it was
		assert_eq!(
			*state.stt_status.lock().unwrap(),
			crate::models::EngineStatus::ready(Some("whisper-small-en")),
			"a stalled slot must not authorize an unclaimed status change"
		);
		assert_eq!(
			*state.llm_status.lock().unwrap(),
			crate::models::EngineStatus::ready(Some("gemma-4-E4B")),
			"a stalled slot must not authorize an unclaimed status change"
		);
		assert!(state.stt_loading.load(std::sync::atomic::Ordering::SeqCst));
		assert!(state.llm_loading.load(std::sync::atomic::Ordering::SeqCst));
		assert!(stt_loads.requests().is_empty() && llm_loads.requests().is_empty());
		assert!(events.snapshot().is_empty(), "{:?}", events.snapshot());
	}

	#[test]
	fn a_secret_only_save_publications_stay_silent_on_ready_engines() {
		// A save that changed nothing about routing (here: only a
		// secret) must not make the loader publish anything itself: its
		// plans are loads of already-working engines, and the
		// already-resident skip is proven at the swap level.
		let (state, _dir) = loader_state("secret-only");
		mark_downloaded(&state, "whisper-small-en", ModelKind::Stt);
		mark_downloaded(&state, "gemma-4-E4B", ModelKind::Llm);
		state
			.mutate_ai_settings(|s| {
				s.stt_engine = SpeechEngine::Whisper;
				s.stt_mode = SttMode::Local;
				s.stt_model = "whisper-small-en".into();
				s.llm_mode = LlmMode::Local;
				s.llm_model = "gemma-4-E4B".into();
				Ok(())
			})
			.expect("commit local routing");
		// ...then the user saves ONLY a secret
		let (captured, generation) = state
			.mutate_ai_settings(|s| {
				s.hf_token = "fake-test-token".into();
				Ok(())
			})
			.expect("secret-only save");
		assert_eq!(captured.hf_token, "fake-test-token");
		// the engines this run reconciles toward are already loaded and
		// ready - what the previous run left behind
		*state.stt_status.lock().unwrap() =
			crate::models::EngineStatus::ready(Some("whisper-small-en"));
		*state.llm_status.lock().unwrap() = crate::models::EngineStatus::ready(Some("gemma-4-E4B"));

		let events = RecordingEvents::new();
		let stt_loads = Arc::new(LoadRecorder::default());
		let llm_loads = Arc::new(LoadRecorder::default());
		let stt_runner = |spec: &models::ModelSpec| stt_loads.record(spec);
		let llm_runner = |spec: &models::ModelSpec| llm_loads.record(spec);
		reconcile_model_state(
			&state,
			&*events,
			&captured,
			generation,
			SHORT_WAIT,
			false,
			&stt_runner,
			&llm_runner,
		);

		assert_eq!(
			*state.stt_status.lock().unwrap(),
			crate::models::EngineStatus::ready(Some("whisper-small-en")),
			"a same-plan run must not republish or disturb the ready engine"
		);
		assert_eq!(
			*state.llm_status.lock().unwrap(),
			crate::models::EngineStatus::ready(Some("gemma-4-E4B")),
			"a same-plan run must not republish or disturb the ready engine"
		);
		assert!(events.snapshot().is_empty(), "{:?}", events.snapshot());
	}

	#[test]
	fn startup_reconciliation_plans_from_committed_settings() {
		// A previous session committed external STT routing; a cold
		// restart must reconcile from exactly those committed settings.
		let dir = tempfile::tempdir().expect("tempdir");
		let db_path = dir.path().join("startup.db");
		{
			let db = crate::db::Db::open(&db_path).expect("db");
			let state = AppState::new(db, dir.path().to_path_buf());
			state
				.mutate_ai_settings(|s| {
					s.stt_mode = SttMode::External;
					s.ext_stt_base_url = "http://localhost:9000".into();
					Ok(())
				})
				.expect("commit external stt");
		}

		let db = crate::db::Db::open(&db_path).expect("reopen db");
		let state = Arc::new(AppState::new(db, dir.path().to_path_buf()));
		let committed = AiSettings::load(&state.db);
		// the startup capture: nothing has mutated the fresh state yet
		let generation = state
			.ai_settings_generation
			.load(std::sync::atomic::Ordering::SeqCst);

		let events = RecordingEvents::new();
		let stt_loads = Arc::new(LoadRecorder::default());
		let llm_loads = Arc::new(LoadRecorder::default());
		let stt_runner = |spec: &models::ModelSpec| stt_loads.record(spec);
		let llm_runner = |spec: &models::ModelSpec| llm_loads.record(spec);
		reconcile_model_state(
			&state,
			&*events,
			&committed,
			generation,
			SHORT_WAIT,
			false,
			&stt_runner,
			&llm_runner,
		);

		// committed external STT: published as external, no whisper load
		assert_eq!(
			*state.stt_status.lock().unwrap(),
			crate::models::EngineStatus::external()
		);
		// the LLM was never downloaded: the committed default reports missing
		assert_eq!(
			*state.llm_status.lock().unwrap(),
			crate::models::EngineStatus::missing()
		);
		assert!(stt_loads.requests().is_empty() && llm_loads.requests().is_empty());
		assert_eq!(
			events.snapshot(),
			vec![
				"stt:External:None".to_string(),
				"llm:Missing:None".to_string(),
			]
		);
	}

	#[test]
	fn auto_apple_with_a_whisper_fallback_reports_apple_as_the_engine() {
		// macOS 26 default: Auto + Apple Speech available + whisper
		// downloaded as the fallback. The whisper load leaves the slot
		// reporting whisper; the run must publish Apple Speech as what
		// actually transcribes.
		let (state, _dir) = loader_state("auto-apple");
		mark_downloaded(&state, "whisper-small-en", ModelKind::Stt);
		let (captured, generation) = state
			.mutate_ai_settings(|s| {
				s.stt_engine = SpeechEngine::Auto;
				s.stt_model = "whisper-small-en".into();
				Ok(())
			})
			.expect("commit auto engine");

		let events = RecordingEvents::new();
		let stt_loads = Arc::new(LoadRecorder::default());
		let llm_loads = Arc::new(LoadRecorder::default());
		let stt_runner = |spec: &models::ModelSpec| stt_loads.record(spec);
		let llm_runner = |spec: &models::ModelSpec| llm_loads.record(spec);
		reconcile_model_state(
			&state,
			&*events,
			&captured,
			generation,
			SHORT_WAIT,
			true,
			&stt_runner,
			&llm_runner,
		);

		assert_eq!(stt_loads.requests(), vec!["whisper-small-en".to_string()]);
		let stt_events: Vec<String> = events
			.snapshot()
			.into_iter()
			.filter(|e| e.starts_with("stt:"))
			.collect();
		assert_eq!(
			stt_events.last().map(String::as_str),
			Some("stt:Ready:Some(\"apple-speech\")"),
			"the last published STT status must say Apple Speech transcribes: {stt_events:?}"
		);
		assert_eq!(
			*state.stt_status.lock().unwrap(),
			crate::models::EngineStatus::ready(Some("apple-speech"))
		);
	}

	#[test]
	fn explicit_apple_on_an_old_macos_publishes_the_whisper_fallback_notice() {
		// explicit Apple Speech on a system without it: whisper stands
		// in, and the status must explain that (not report whisper as
		// the chosen engine).
		let (state, _dir) = loader_state("apple-unsupported");
		mark_downloaded(&state, "whisper-small-en", ModelKind::Stt);
		let (captured, generation) = state
			.mutate_ai_settings(|s| {
				s.stt_engine = SpeechEngine::Apple;
				s.stt_mode = SttMode::Local;
				s.stt_model = "whisper-small-en".into();
				s.ext_stt_base_url = String::new();
				Ok(())
			})
			.expect("commit apple engine");

		let events = RecordingEvents::new();
		let stt_loads = Arc::new(LoadRecorder::default());
		let llm_loads = Arc::new(LoadRecorder::default());
		let stt_runner = |spec: &models::ModelSpec| stt_loads.record(spec);
		let llm_runner = |spec: &models::ModelSpec| llm_loads.record(spec);
		reconcile_model_state(
			&state,
			&*events,
			&captured,
			generation,
			SHORT_WAIT,
			false,
			&stt_runner,
			&llm_runner,
		);

		assert_eq!(stt_loads.requests(), vec!["whisper-small-en".to_string()]);
		let snapshot = events.snapshot();
		assert!(
			snapshot
				.iter()
				.any(|e| e == "stt:Error:Some(\"apple-speech\")"),
			"the error status must be published: {snapshot:?}"
		);
		assert_eq!(
			*state.stt_status.lock().unwrap(),
			crate::models::EngineStatus::error(
				Some("apple-speech"),
				"Apple Speech requires macOS 26+ - using whisper instead",
			)
		);
	}
}
