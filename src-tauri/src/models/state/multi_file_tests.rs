//! Multi-file (ONNX) models: "downloaded" means every file, and the
//! loader gets one folder holding all of them.

use super::AppState;
use crate::models::{find_model, ModelKind, ModelSpec};

fn temp_state(name: &str) -> (AppState, tempfile::TempDir) {
	let dir = tempfile::tempdir().expect("tempdir");
	std::fs::create_dir_all(dir.path().join("models")).expect("models dir");
	let db = crate::db::Db::open(&dir.path().join(format!("{name}.db"))).expect("db");
	(AppState::new(db, dir.path().to_path_buf()), dir)
}

fn parakeet() -> ModelSpec {
	find_model("parakeet-tdt-0.6b-v3", ModelKind::Stt)
		.expect("catalog model")
		.clone()
}

#[test]
fn a_multi_file_model_is_downloaded_only_with_every_companion() {
	let (state, _dir) = temp_state("multi-downloaded");
	let spec = parakeet();
	assert!(!spec.companions.is_empty());
	std::fs::write(state.model_path(&spec), b"encoder").expect("main file");
	assert!(
		!state.is_model_downloaded(&spec),
		"the encoder alone is not a usable model"
	);
	for companion in spec.companion_specs() {
		std::fs::write(state.model_path(&companion), b"part").expect("companion");
	}
	assert!(state.is_model_downloaded(&spec));
}

#[test]
fn the_model_folder_is_the_main_files_folder_when_everything_is_there() {
	let (state, _dir) = temp_state("multi-dir");
	let spec = parakeet();
	for file in spec.all_file_specs() {
		std::fs::write(state.model_path(&file), b"x").expect("file");
	}
	assert_eq!(
		state.model_dir_for(&spec, &state.model_path(&spec)),
		Some(state.models_dir())
	);
}

#[test]
fn scattered_files_are_linked_into_one_folder() {
	let (state, dir) = temp_state("multi-links");
	let spec = parakeet();
	for companion in spec.companion_specs() {
		std::fs::write(state.model_path(&companion), companion.filename).expect("companion");
	}
	// the main file resolved somewhere else (another tool's cache layout)
	let elsewhere = dir.path().join("elsewhere");
	std::fs::create_dir_all(&elsewhere).expect("dir");
	let main = elsewhere.join(spec.filename);
	std::fs::write(&main, b"encoder").expect("main file");

	let folder = state.model_dir_for(&spec, &main).expect("a link folder");
	assert_eq!(folder, state.models_dir().join(spec.id));
	for file in spec.all_file_specs() {
		assert!(
			folder.join(file.filename).is_file(),
			"{} linked",
			file.filename
		);
	}
	assert_eq!(
		std::fs::read(folder.join(spec.filename)).unwrap(),
		b"encoder"
	);
	// linking again (a reload) replaces the old links
	assert_eq!(state.model_dir_for(&spec, &main), Some(folder));
}

#[test]
fn a_missing_companion_leaves_no_folder_to_load() {
	let (state, _dir) = temp_state("multi-missing");
	let spec = parakeet();
	std::fs::write(state.model_path(&spec), b"encoder").expect("main file");
	assert_eq!(state.model_dir_for(&spec, &state.model_path(&spec)), None);
}
