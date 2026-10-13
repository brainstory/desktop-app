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

/// An activation/load starting at the delete boundary must be
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

/// A filesystem failure after the unload must still publish a
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
/// bulldozing past it (acquire-abort semantics on a wedged slot).
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

/// The download guard is re-checked UNDER the claim, so a
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
/// no mutation: the holder keeps its slot (acquire-abort semantics).
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
	std::fs::write(foreign.join(spec.filename), b"someone else's revision").expect("foreign file");

	let cache_path = cache.path().to_path_buf();
	let delete_files = || {
		let mut deleted = false;
		if app_path.is_file() {
			std::fs::remove_file(&app_path).map_err(|e| e.to_string())?;
			deleted = true;
		}
		deleted |= crate::models::remove_cached_model(&cache_path, &spec, &Default::default())?;
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
