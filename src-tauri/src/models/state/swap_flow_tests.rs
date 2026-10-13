use super::{AppState, EngineSlot, EngineState, EngineStatus};
use crate::StatusEvents;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

/// A stand-in engine: nothing whisper/llama-shaped, just the id (and,
/// for the LLM-shaped tests, the ctx it was built at) the swap flow
/// reads.
struct FakeEngine {
	id: String,
	built_ctx: u32,
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

/// The trained context the fake "GGUF" reports; the gemma catalog
/// models are trained to 128k+.
const FAKE_TRAINED_CTX: u32 = 131072;

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

fn llm_spec(id: &str) -> super::super::catalog::ModelSpec {
	super::super::catalog::find_model(id, super::super::catalog::ModelKind::Llm)
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
			|engine| model_id_of(engine) == spec.id,
			|| Ok(()),
			|_path| {
				Ok(FakeEngine {
					id: spec.id.to_string(),
					built_ctx: 0,
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
				|engine| model_id_of(engine) == spec.id,
				|| Ok(()),
				|_path| {
					factory_called.store(true, Ordering::SeqCst);
					loaded_tx.send(()).expect("test alive");
					// the multi-second engine load: the newer save
					// commits before this returns
					release_rx.lock().unwrap().recv().expect("released");
					Ok(FakeEngine {
						id: spec.id.to_string(),
						built_ctx: 0,
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
		built_ctx: 0,
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
		|engine| model_id_of(engine) == spec.id,
		|| Ok(()),
		|_path| {
			factory_called.store(true, Ordering::SeqCst);
			Ok(FakeEngine {
				id: spec.id.to_string(),
				built_ctx: 0,
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
		built_ctx: 0,
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
		|engine| model_id_of(engine) == spec.id,
		|| Ok(()),
		|_path| Err("corrupt file".into()),
		|restore| {
			rollbacks.lock().unwrap().push(restore.id.to_string());
			slot.install(Arc::new(FakeEngine {
				id: restore.id.to_string(),
				built_ctx: 0,
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
		|engine| model_id_of(engine) == spec.id,
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

/// The production LLM already-loaded predicate against the fake
/// engine: same model id AND the resident engine was built at the
/// window the CURRENT settings resolve to.
fn llm_unchanged<'a>(
	state: &'a AppState,
	spec_id: &'static str,
) -> impl Fn(&FakeEngine) -> bool + 'a {
	move |engine| {
		engine.id.as_str() == spec_id
			&& engine.built_ctx
				== crate::llm::effective_ctx_for(
					state.ai_settings().llm_ctx_tokens,
					Some(FAKE_TRAINED_CTX),
				)
	}
}

#[test]
fn a_same_model_load_at_the_same_resolved_ctx_does_not_rebuild() {
	let (state, _dir) = temp_state("swap-ctx-same");
	let spec = llm_spec("gemma-4-E4B");
	// resident engine was built at the default window (requested 0)
	let resident = Arc::new(FakeEngine {
		id: spec.id.to_string(),
		built_ctx: crate::llm::effective_ctx_for(0, Some(FAKE_TRAINED_CTX)),
	});
	let slot = FakeSlot::empty();
	slot.install(resident.clone());
	let status = Mutex::new(EngineStatus::ready(Some(spec.id)));
	let events = RecordingEvents::new();
	let factory_called = AtomicBool::new(false);

	// settings still request the default window: nothing to do
	let result = state.swap_engine(
		&events,
		&spec,
		&status,
		crate::notify_llm,
		&slot,
		model_id_of,
		llm_unchanged(&state, spec.id),
		|| Ok(()),
		|_path| {
			factory_called.store(true, Ordering::SeqCst);
			Ok(FakeEngine {
				id: spec.id.to_string(),
				built_ctx: 0,
			})
		},
		|_prev| panic!("no load happens: no rollback"),
		false,
		true,
		|| true,
	);

	result.expect("the resident engine already matches");
	assert!(
		!factory_called.load(Ordering::SeqCst),
		"a same id + same resolved ctx must not reload the engine"
	);
	assert!(Arc::ptr_eq(
		&slot.installed().expect("engine kept"),
		&resident
	));
	assert_eq!(*status.lock().unwrap(), EngineStatus::ready(Some(spec.id)));
	assert!(events.snapshot().is_empty(), "{:?}", events.snapshot());
}

#[test]
fn a_ctx_only_save_rebuilds_the_same_model_at_the_new_window() {
	let (state, _dir) = temp_state("swap-ctx-change");
	let spec = llm_spec("gemma-4-E4B");
	// the install step checks the file still exists on disk
	std::fs::write(state.model_path(&spec), b"stub").expect("model file");
	// the engine currently resident was built at the default window
	let slot = FakeSlot::with(FakeEngine {
		id: spec.id.to_string(),
		built_ctx: crate::llm::effective_ctx_for(0, Some(FAKE_TRAINED_CTX)),
	});
	let status = Mutex::new(EngineStatus::ready(Some(spec.id)));
	let events = RecordingEvents::new();

	// the user saves ONLY a context-window change; the loader then
	// runs for the same model id
	state
		.mutate_ai_settings(|s| s.apply_updates(&serde_json::json!({ "llmCtxTokens": 65536 })))
		.expect("ctx-only save commits");

	// the requested window each factory invocation was asked to build
	let built_at: Mutex<Vec<u32>> = Mutex::new(Vec::new());
	state
		.swap_engine(
			&events,
			&spec,
			&status,
			crate::notify_llm,
			&slot,
			model_id_of,
			llm_unchanged(&state, spec.id),
			|| Ok(()),
			|_path| {
				// like the production factory, the requested window is
				// read from the CURRENT settings
				let requested = state.ai_settings().llm_ctx_tokens;
				built_at.lock().unwrap().push(requested);
				Ok(FakeEngine {
					id: spec.id.to_string(),
					built_ctx: crate::llm::effective_ctx_for(requested, Some(FAKE_TRAINED_CTX)),
				})
			},
			|_prev| panic!("a successful rebuild never rolls back"),
			false,
			true,
			|| true,
		)
		.expect("the rebuild installs");

	assert_eq!(
		*built_at.lock().unwrap(),
		vec![65536],
		"the factory was asked to build the settings' window"
	);
	assert_eq!(
		slot.installed().expect("rebuilt engine").built_ctx,
		65536,
		"the engine is resident at the new window"
	);
	assert_eq!(*status.lock().unwrap(), EngineStatus::ready(Some(spec.id)));
}
