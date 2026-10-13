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
