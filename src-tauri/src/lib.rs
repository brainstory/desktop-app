pub mod app;
pub mod apple;
mod commands;
pub mod db;
pub mod denoise;
pub mod keys;
pub mod llm;
pub mod models;
mod notify;
pub mod prompts;
pub mod reactions;
mod reminders;
pub mod secrets;
pub mod stt;
pub mod stt_apple;
pub mod types;
pub mod vad;
pub mod voice;

use chrono::Utc;
use tauri::{Manager, WindowEvent, Wry};

use models::loader::{
	notify_llm, notify_stt, plan_model_load, spawn_migration_and_model_loader, spawn_model_loader,
	AppStatusEvents, LlmPlan, StatusEvents, WhisperPlan,
};
use models::{AiSettings, AppState};

// re-imported at the root so sibling modules keep their crate::-paths
// after the app-shell and loader code moved out of this file
use app::{
	apply_presence, open_new_idea, quick_capture_enabled, register_quick_capture_shortcut,
	setup_tray, shutdown_and_exit, sync_tray_reminder_check, QUICK_CAPTURE_SHORTCUT,
};

/// One-time app setup: data dir, database, managed state, tray,
/// reminders and the startup model load. Split out of [`run`] so
/// rustfmt covers the body (it silently skips oversized closures).
fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
	let data_dir = match app.path().app_data_dir() {
		Ok(dir) => dir,
		Err(e) => {
			// Without the data dir there is nowhere to open (or
			// quarantine) the database; tell the user instead of
			// panicking invisibly behind a GUI launch.
			log::error!("failed to resolve app data dir: {e}");
			{
				use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
				app.dialog()
					.message(format!(
						"Brainstory could not locate its data directory and cannot start.\n\n{e}"
					))
					.title("Brainstory")
					.kind(MessageDialogKind::Error)
					.blocking_show();
			}
			return Err(format!("failed to resolve app data dir: {e}").into());
		}
	};
	std::fs::create_dir_all(data_dir.join("models")).ok();
	// Secrets can fall back to plaintext rows in this folder when
	// no keychain is available: keep it owner-only (best effort).
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let _ = std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700));
	}
	// Drop only .part files that can never be resumed; a download
	// interrupted by a quit continues from its .part next time.
	models::sweep_stale_part_files(&data_dir.join("models"), &models::primary_hub_cache());
	prompts::init_overrides(&data_dir);
	llm::sampling::init_overrides(&data_dir);

	let db = match open_database(&data_dir) {
		Ok(db) => db,
		Err(message) => {
			// The app cannot start without its data store; a native
			// dialog is the only way to tell the user why nothing
			// launched. setup() returning Err aborts the launch.
			log::error!("{message}");
			{
				use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
				app.dialog()
							.message(format!(
								"Brainstory could not open its data store and cannot start.\n\n{message}\n\nYour data directory:\n{}",
								data_dir.display()
							))
							.title("Brainstory")
							.kind(MessageDialogKind::Error)
							.blocking_show();
			}
			return Err(message.into());
		}
	};
	// Move any plaintext secrets from early builds into the keychain.
	secrets::migrate_from_db(&db);
	// Persist the speech-engine default for fresh installs once,
	// here: resolving a settings *read* must never write.
	if db.get_setting(keys::setting::AI_STT_ENGINE).is_none() {
		let default = models::default_stt_engine(&db);
		if let Err(e) = db.set_setting(keys::setting::AI_STT_ENGINE, default.as_str()) {
			log::warn!("failed to persist default speech engine: {e}");
		}
	}
	if db.get_setting(keys::setting::CREATED_AT).is_none() {
		// naive UTC, no trailing Z (the frontend appends it itself)
		let now = Utc::now()
			.naive_utc()
			.format("%Y-%m-%dT%H:%M:%S")
			.to_string();
		if let Err(e) = db.set_setting(keys::setting::CREATED_AT, &now) {
			log::error!("failed to record account creation date: {e}");
		}
	}

	app.manage(AppState::new(db, data_dir));

	// the preferred microphone, read once (an unplugged device
	// falls back to the default inside voice.rs)
	if let Some(preferred) = app
		.state::<AppState>()
		.db
		.get_setting(keys::setting::INPUT_DEVICE)
	{
		voice::set_preferred_device(&preferred);
	}

	setup_tray(app)?;
	// Quick capture is on by default; a stored "false" releases
	// the global shortcut for another app (settings can re-enable).
	if quick_capture_enabled(&app.state::<AppState>().db) {
		register_quick_capture_shortcut(app.handle());
	}

	// Apply the dock/tray visibility preferences from settings.
	{
		let state = app.state::<AppState>();
		let dock = state
			.db
			.get_setting(keys::setting::SHOW_IN_DOCK)
			.map(|v| v == "true")
			.unwrap_or(true);
		let tray = state
			.db
			.get_setting(keys::setting::SHOW_IN_TRAY)
			.map(|v| v == "true")
			.unwrap_or(true);
		apply_presence(app.handle(), dock, tray);
	}

	reminders::spawn(app.handle().clone());

	// Dev-only startup diagnostic: after the onboarding redirect
	// settles, a JS error would have renamed the window title (see
	// the on_page_load error hook). Never ships in release builds.
	if cfg!(debug_assertions) {
		if let Some(window) = app.get_webview_window("main") {
			std::thread::spawn(move || {
				std::thread::sleep(std::time::Duration::from_secs(6));
				let url = window
					.url()
					.map(|u| u.to_string())
					.unwrap_or_else(|e| format!("<url err: {e}>"));
				let title = window
					.title()
					.unwrap_or_else(|e| format!("<title err: {e}>"));
				log::info!("[startup check] url={url} title={title}");
			});
		}
	}
	// Read the generation BEFORE the settings: a save committing
	// between the two reads must leave this run stale (it defers to
	// the save's own loader) rather than plan from a superseded
	// snapshot it mistakes for current.
	let startup_generation = app
		.state::<AppState>()
		.ai_settings_generation
		.load(std::sync::atomic::Ordering::SeqCst);
	spawn_migration_and_model_loader(
		app.handle().clone(),
		AiSettings::load(&app.state::<AppState>().db),
		startup_generation,
	);

	Ok(())
}

/// The plugin stack, split out of [`run`] so the builder chain stays
/// short enough for rustfmt to format (it silently gives up past a
/// nesting/size budget, which let mis-indentation hide in the old
/// monolithic chain).
fn install_plugins(builder: tauri::Builder<Wry>) -> tauri::Builder<Wry> {
	builder
		.plugin(
			// Must be the first plugin: without a registered logger every
			// log::error!/warn! in the app is silently discarded, and
			// model-load/DB diagnostics are the difference between a
			// debuggable report and a mystery.
			tauri_plugin_log::Builder::new()
				.targets([
					tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
					tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
						file_name: Some("brainstory".into()),
					}),
				])
				.level(log::LevelFilter::Info)
				// generous retention: diagnosing a model-load failure a
				// day later needs more than one 512 KB file
				.max_file_size(2_000_000)
				.rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
				.build(),
		)
		.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
			// A second launch just focuses the existing window instead of
			// running two processes against the same database.
			if let Some(window) = app.get_webview_window("main") {
				let _ = window.show();
				let _ = window.unminimize();
				let _ = window.set_focus();
			}
		}))
		.plugin(tauri_plugin_notification::init())
		.plugin(tauri_plugin_dialog::init())
		.plugin(tauri_plugin_updater::Builder::new().build())
		.plugin(tauri_plugin_process::init())
		// remember the window's size and position across launches
		.plugin(tauri_plugin_window_state::Builder::default().build())
		// opening folders/files in the system file manager (Rust-side
		// only; the capability file stays narrow)
		.plugin(tauri_plugin_opener::init())
		.plugin(
			// Quick capture: the shortcut handler fires even while the
			// window is hidden in the tray, so an idea can be started
			// without hunting for the app first.
			tauri_plugin_global_shortcut::Builder::new()
				.with_handler(|app, _shortcut, event| {
					if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
						open_new_idea(app);
					}
				})
				.build(),
		)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	install_plugins(tauri::Builder::default())
		.invoke_handler(tauri::generate_handler![
			commands::data::get_user,
			commands::data::get_daily_status,
			commands::data::link_idea_log,
			commands::data::open_data_folder,
			commands::data::set_idea_result,
			commands::data::get_input_device,
			commands::data::list_input_devices,
			commands::data::set_input_device,
			commands::data::test_input_device,
			commands::data::open_log_folder,
			commands::data::copy_diagnostics,
			commands::data::get_all_ideas,
			commands::data::get_idea,
			commands::data::get_idea_children,
			commands::data::create_idea,
			commands::data::update_idea,
			commands::data::mark_idea_read,
			commands::data::delete_idea,
			commands::data::add_idea_note,
			commands::data::update_idea_note,
			commands::data::delete_idea_note,
			commands::data::get_log_questions,
			commands::data::submit_log,
			commands::data::get_survey_fields,
			commands::data::submit_survey,
			commands::data::get_notifications,
			commands::ai::transcribe,
			commands::ai::generate_streaming_response,
			commands::ai::regenerate_result,
			commands::ai::cancel_generation,
			commands::ai::cancel_transcription,
			commands::ai::stop_and_transcribe,
			commands::ai::send_test_notification,
			commands::ai::start_voice_capture,
			commands::ai::stop_voice_capture,
			commands::settings::get_user_settings,
			commands::settings::save_user_settings,
			commands::settings::set_app_presence,
			commands::settings::set_quick_capture_enabled,
			commands::settings::get_updates_enabled,
			commands::settings::set_updates_enabled,
			commands::settings::get_ai_settings,
			commands::settings::save_ai_settings,
			commands::settings::test_llm_endpoint,
			commands::settings::test_stt_endpoint,
			commands::models_cmd::list_models,
			commands::models_cmd::get_runtime_status,
			commands::models_cmd::get_apple_stt_status,
			commands::models_cmd::get_free_disk_space,
			commands::models_cmd::cancel_model_load,
			commands::models_cmd::download_model,
			commands::models_cmd::cancel_download,
			commands::models_cmd::pause_download,
			commands::models_cmd::delete_model,
			commands::models_cmd::activate_model,
			commands::share::export_idea,
			commands::share::export_markdown,
			commands::share::import_share,
			commands::reactions::get_reactions,
			commands::reactions::toggle_section_reaction,
			commands::reactions::toggle_comment_reaction,
		])
		.on_page_load(|webview, payload| {
			match payload.event() {
				tauri::webview::PageLoadEvent::Started => {
					log::info!("page load started: {}", payload.url());
					// A new page load discards the previous page (Astro
					// full-document navigation, reload): its capture belongs
					// to a dead page, so release the microphone and abandon
					// the audio - it must never be transcribed. No-op when
					// nothing is recording (every ordinary page load).
					voice::abandon_capture();
				}
				tauri::webview::PageLoadEvent::Finished => {
					log::info!("page load finished: {}", payload.url());
				}
			}
			// Dev-only diagnostic: surface webview JS errors in the window
			// title so a crashed page is diagnosable from the log instead
			// of being just a white rectangle. Debug hooks do not ship in
			// release builds.
			if cfg!(debug_assertions) {
				let _ = webview.eval(
					"window.addEventListener('error', function(e){ document.title = 'JSERR: ' + e.message; });\n\
					 window.addEventListener('unhandledrejection', function(e){ document.title = 'PROMISE-REJ: ' + e.reason; });",
				);
			}
		})
		.setup(setup)
		.on_window_event(|window, event| {
			match event {
				WindowEvent::CloseRequested { api, .. } => {
					let app = window.app_handle();
					let quit_on_close = app
						.state::<AppState>()
						.quit_on_close
						.load(std::sync::atomic::Ordering::Relaxed);
					if quit_on_close {
						// tray hidden: closing the window is the way out
						crate::shutdown_and_exit(app);
					} else {
						// Closing the window hides it to the tray so daily
						// reminders keep working; quit via the tray menu.
						// Deliberately NOT abandoning a capture here: the
						// page stays alive, so the recorder hook's timer
						// still fires and voice::MAX_CAPTURE_SECS caps the
						// buffer while the window is hidden.
						window.hide().ok();
						api.prevent_close();
					}
				}
				WindowEvent::Destroyed => {
					// The webview is gone for good, so its page can never
					// run cleanup JS: release the microphone and discard
					// the recording (never transcribed).
					voice::abandon_capture();
				}
				_ => {}
			}
		})
		.build(tauri::generate_context!())
		.unwrap_or_else(|e| {
			// No app exists yet, so there is no window or dialog to show;
			// exit cleanly with the reason on stderr instead of panicking.
			eprintln!("error while building brainstory: {e}");
			std::process::exit(1);
		})
		.run(|app, event| {
			// Every quit path (tray menu, Cmd+Q, dock quit, logout) funnels
			// through here before AppKit calls exit(). _exit() skips C++
			// static destructors, which abort on the vendored ggml teardown.
			if let tauri::RunEvent::Exit = event {
				// release the microphone on the way out
				voice::abandon_capture();
				crate::shutdown_and_exit(app);
			}
		});
}

/// Open the database, quarantining a *corrupt* file instead of failing to
/// launch forever. The old file is kept for manual recovery. Any other
/// open failure (permissions, disk full, ...) surfaces as Err - renaming
/// the user's database away on a transient error would look like a
/// factory reset.
fn open_database(data_dir: &std::path::Path) -> Result<db::Db, String> {
	let db_path = data_dir.join("brainstory.db");
	match db::Db::open(&db_path) {
		Ok(db) => Ok(db),
		Err(db::OpenError::Sqlite(e)) if is_db_corruption(&e) => {
			log::error!("database is corrupt ({e}); quarantining it and starting fresh");
			let stamp = Utc::now().format("%Y%m%d-%H%M%S");
			let corrupt = data_dir.join(format!("brainstory.db.corrupt-{stamp}"));
			if let Err(rename_err) = std::fs::rename(&db_path, &corrupt) {
				log::error!("failed to quarantine the corrupt database: {rename_err}");
			}
			// move WAL sidecars along with it so the fresh DB starts clean.
			// SQLite names them <db>-wal / <db>-shm (not <stem>.wal), and
			// with_extension would also mangle the quarantine stamp.
			for suffix in ["-wal", "-shm"] {
				let _ = std::fs::rename(
					append_file_suffix(&db_path, suffix),
					append_file_suffix(&corrupt, suffix),
				);
			}
			db::Db::open(&db_path).map_err(|e| {
				format!(
					"the database was quarantined as corrupt, but a fresh database could not be created either: {e}"
				)
			})
		}
		Err(e) => Err(format!("could not open the database: {e}")),
	}
}

/// `path` with `suffix` appended to its file name
/// (`dir/brainstory.db` + `-wal` -> `dir/brainstory.db-wal`).
fn append_file_suffix(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
	let mut name = path
		.file_name()
		.map(|n| n.to_os_string())
		.unwrap_or_default();
	name.push(suffix);
	path.with_file_name(name)
}

/// True only for errors that actually indicate a corrupt/unreadable file -
/// not for transient failures like SQLITE_BUSY or a full disk.
fn is_db_corruption(e: &rusqlite::Error) -> bool {
	use rusqlite::ffi::ErrorCode;
	matches!(
		e.sqlite_error_code(),
		Some(ErrorCode::DatabaseCorrupt) | Some(ErrorCode::NotADatabase)
	)
}

#[cfg(test)]
mod tests {
	use super::open_database;

	#[test]
	fn open_database_quarantines_corrupt_file_and_sidecars() {
		let temp = tempfile::tempdir().expect("tempdir");
		let dir = temp.path().to_path_buf();
		let db_path = dir.join("brainstory.db");
		// a real WAL-mode database truncated mid-page opens with
		// SQLITE_CORRUPT (not NOTADB), so the quarantine path runs
		{
			let conn = rusqlite::Connection::open(&db_path).unwrap();
			conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(a);")
				.unwrap();
		}
		let data = std::fs::read(&db_path).unwrap();
		assert!(data.len() > 512, "expected a multi-page database");
		std::fs::write(&db_path, &data[..512]).unwrap();
		let marker = b"STALE SIDECAR MARKER";
		std::fs::write(dir.join("brainstory.db-wal"), marker).unwrap();
		std::fs::write(dir.join("brainstory.db-shm"), marker).unwrap();

		let db = open_database(&dir).expect("quarantine the corrupt file and start fresh");
		drop(db);

		// a fresh database exists at the canonical path
		assert!(db_path.is_file(), "a fresh brainstory.db must exist");

		// the corrupt main file moved to a stamped quarantine name
		let names: Vec<String> = std::fs::read_dir(&dir)
			.unwrap()
			.filter_map(|e| e.ok())
			.map(|e| e.file_name().to_string_lossy().into_owned())
			.collect();
		assert!(
			names
				.iter()
				.any(|n| n.starts_with("brainstory.db.corrupt-")),
			"quarantined main file present: {names:?}"
		);

		// Stale sidecar content must never survive next to the fresh
		// database, whichever mechanism removed it (SQLite's own
		// close-time cleanup during the failed open, or the quarantine
		// renames that run when that best-effort cleanup fails - locked
		// files, Windows, ...). A fresh brainstory.db-wal may exist, but
		// it must not contain the old bytes.
		for entry in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
			let bytes = std::fs::read(entry.path()).unwrap();
			assert!(
				!bytes.windows(marker.len()).any(|w| w == marker),
				"stale sidecar content survived in {}",
				entry.path().display()
			);
		}
	}

	#[test]
	fn append_file_suffix_builds_sqlite_sidecar_names() {
		let dir = std::path::Path::new("/tmp");
		let db = dir.join("brainstory.db");
		// SQLite sidecars append to the full file name; with_extension
		// would produce brainstory.wal instead of brainstory.db-wal
		assert_eq!(
			super::append_file_suffix(&db, "-wal"),
			dir.join("brainstory.db-wal")
		);
		assert_eq!(
			super::append_file_suffix(&db, "-shm"),
			dir.join("brainstory.db-shm")
		);
		// the quarantine stamp must survive too (with_extension would
		// collapse brainstory.db.corrupt-STAMP-wal to brainstory.db.wal)
		let stamped = dir.join("brainstory.db.corrupt-20260101-000000");
		assert_eq!(
			super::append_file_suffix(&stamped, "-wal"),
			dir.join("brainstory.db.corrupt-20260101-000000-wal")
		);
	}
}
