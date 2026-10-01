pub mod apple;
mod commands;
pub mod db;
pub mod keys;
pub mod llm;
pub mod models;
pub mod prompts;
pub mod reactions;
mod reminders;
pub mod secrets;
pub mod stt;
pub mod stt_apple;
pub mod types;
pub mod voice;

use std::sync::Mutex;

use chrono::Utc;
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent, Wry};

use models::{AiSettings, AppState, ModelKind};

/// Handle to the tray's reminder check item, kept so settings-page changes
/// can sync its checkmark (otherwise it shows the opposite of reality).
static TRAY_REMINDER_ITEM: Mutex<Option<CheckMenuItem<Wry>>> = Mutex::new(None);

pub fn sync_tray_reminder_check(enabled: bool) {
	let guard = TRAY_REMINDER_ITEM.lock().unwrap_or_else(|e| e.into_inner());
	if let Some(item) = guard.as_ref() {
		if let Err(e) = item.set_checked(enabled) {
			log::warn!("failed to update tray reminder checkmark: {e}");
		}
	}
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	tauri::Builder::default()
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
				.max_file_size(512_000)
				.rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
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
		.invoke_handler(tauri::generate_handler![
			commands::data::get_user,
			commands::data::get_daily_status,
			commands::data::get_all_ideas,
			commands::data::get_idea,
			commands::data::get_idea_children,
			commands::data::create_idea,
			commands::data::update_idea,
			commands::data::mark_idea_read,
			commands::data::delete_idea,
			commands::data::get_log_questions,
			commands::data::submit_log,
			commands::data::get_survey_fields,
			commands::data::submit_survey,
			commands::data::get_notifications,
			commands::ai::transcribe,
			commands::ai::generate_response,
			commands::ai::generate_streaming_response,
			commands::ai::cancel_generation,
			commands::ai::send_test_notification,
			commands::ai::start_voice_capture,
			commands::ai::stop_voice_capture,
			commands::settings::get_user_settings,
			commands::settings::save_user_settings,
			commands::settings::set_app_presence,
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
			commands::models_cmd::download_model,
			commands::models_cmd::cancel_download,
			commands::models_cmd::delete_model,
			commands::models_cmd::activate_model,
			commands::share::export_idea,
			commands::share::import_share,
			commands::reactions::get_reactions,
			commands::reactions::toggle_section_reaction,
			commands::reactions::toggle_comment_reaction,
		])
		.on_page_load(|webview, payload| {
			match payload.event() {
				tauri::webview::PageLoadEvent::Started => {
					log::info!("page load started: {}", payload.url());
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
		.setup(|app| {
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
			// Drop only .part files that can never be resumed; a download
			// interrupted by a quit continues from its .part next time.
			models::sweep_stale_part_files(&data_dir.join("models"), &models::primary_hub_cache());
			prompts::init_overrides(&data_dir);

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

			setup_tray(app)?;

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
						let title = window.title().unwrap_or_else(|e| format!("<title err: {e}>"));
						log::info!("[startup check] url={url} title={title}");
					});
				}
			}
			spawn_migration_and_model_loader(
				app.handle().clone(),
				AiSettings::load(&app.state::<AppState>().db),
			);

			Ok(())
		})
		.on_window_event(|window, event| {
			if let WindowEvent::CloseRequested { api, .. } = event {
				let app = window.app_handle();
				let quit_on_close = app
					.state::<AppState>()
					.quit_on_close
					.load(std::sync::atomic::Ordering::Relaxed);
				if quit_on_close {
					// tray hidden: closing the window is the way out
					crate::force_exit();
				} else {
					// Closing the window hides it to the tray so daily
					// reminders keep working; quit via the tray menu.
					window.hide().ok();
					api.prevent_close();
				}
			}
		})
		.build(tauri::generate_context!())
		.unwrap_or_else(|e| {
			// No app exists yet, so there is no window or dialog to show;
			// exit cleanly with the reason on stderr instead of panicking.
			eprintln!("error while building brainstory: {e}");
			std::process::exit(1);
		})
		.run(|_app, event| {
			// Every quit path (tray menu, Cmd+Q, dock quit, logout) funnels
			// through here before AppKit calls exit(). _exit() skips C++
			// static destructors, which abort on the vendored ggml teardown.
			if let tauri::RunEvent::Exit = event {
				crate::force_exit();
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

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
	let reminder_enabled = app
		.state::<AppState>()
		.db
		.get_setting(keys::setting::REMINDER_ENABLED)
		.map(|v| v == "true")
		.unwrap_or(false);

	let open = MenuItem::with_id(app, "open", "Open Brainstory", true, None::<&str>)?;
	let reminder = CheckMenuItem::with_id(
		app,
		"reminder",
		"Daily reminder",
		true,
		reminder_enabled,
		None::<&str>,
	)?;
	let quit = MenuItem::with_id(app, "quit", "Quit Brainstory", true, None::<&str>)?;
	let menu = Menu::with_items(app, &[&open, &reminder, &quit])?;

	*TRAY_REMINDER_ITEM.lock().unwrap_or_else(|e| e.into_inner()) = Some(reminder);

	let mut tray = TrayIconBuilder::with_id("main-tray")
		.menu(&menu)
		.show_menu_on_left_click(false)
		.tooltip("Brainstory")
		.on_menu_event(|app, event| match event.id().as_ref() {
			"open" => show_main_window(app),
			"reminder" => {
				let state = app.state::<AppState>();
				let enabled = !state
					.db
					.get_setting(keys::setting::REMINDER_ENABLED)
					.map(|v| v == "true")
					.unwrap_or(false);
				if let Err(e) = state.db.set_setting(
					keys::setting::REMINDER_ENABLED,
					if enabled { "true" } else { "false" },
				) {
					log::error!("failed to save reminder setting: {e}");
				}
				// keep the native checkmark and the setting in lockstep
				sync_tray_reminder_check(enabled);
				// wake the scheduler so the toggle applies immediately
				reminders::notify_settings_changed();
			}
			"quit" => {
				crate::force_exit();
			}
			_ => {}
		})
		.on_tray_icon_event(|tray, event| {
			if let TrayIconEvent::Click {
				button: MouseButton::Left,
				button_state: MouseButtonState::Up,
				..
			} = event
			{
				show_main_window(tray.app_handle());
			}
		});

	if let Some(icon) = app.default_window_icon() {
		tray = tray.icon(icon.clone());
	}
	tray.build(app)?;
	Ok(())
}

/// Show/hide the dock icon and tray icon per user settings. When the tray is
/// hidden, closing the window quits the app so it can't get stranded running
/// invisibly in the background.
pub fn apply_presence(app: &AppHandle, dock: bool, tray: bool) {
	{
		let state = app.state::<AppState>();
		state
			.quit_on_close
			.store(!tray, std::sync::atomic::Ordering::Relaxed);
	}

	#[cfg(target_os = "macos")]
	{
		if let Err(e) = app.set_dock_visibility(dock) {
			log::warn!("failed to set dock visibility: {e}");
		}
		if !dock {
			// Resigning the regular activation policy can bounce focus to
			// Finder; take it back so the app stays front and center.
			match objc2::MainThreadMarker::new() {
				Some(mtm) => {
					objc2_app_kit::NSApplication::sharedApplication(mtm).activate();
				}
				None => log::warn!("apply_presence ran off the main thread; not re-activating"),
			}
		}
	}
	#[cfg(not(target_os = "macos"))]
	let _ = dock;

	if let Some(tray_icon) = app.tray_by_id("main-tray") {
		if let Err(e) = tray_icon.set_visible(tray) {
			log::warn!("failed to set tray visibility: {e}");
		}
	}
}

/// Terminate without running C++ static destructors. whisper.cpp and
/// llama.cpp each vendor a ggml copy; their atexit teardown aborts
/// (SIGABRT, the macOS "crashed" dialog). SQLite is WAL-durable, so
/// skipping finalization is safe. The log is flushed first: _exit skips
/// the Rust runtime teardown that would otherwise drain the buffer, and
/// the tail of the log file is usually the interesting part.
pub fn force_exit() -> ! {
	log::logger().flush();
	unsafe { libc::_exit(0) }
}

fn show_main_window(app: &AppHandle) {
	if let Some(window) = app.get_webview_window("main") {
		window.show().ok();
		window.unminimize().ok();
		window.set_focus().ok();
	}
}

/// Load active models at startup / after settings changes. Heavy loading
/// happens on a background thread so the UI starts instantly.
pub fn spawn_model_loader(app: AppHandle, settings: AiSettings) {
	std::thread::spawn(move || run_model_loader(app, settings));
}

/// Startup path: migrate legacy app-dir downloads into the hub cache
/// first (hash-verified, so a corrupt file never poisons a
/// content-addressed store), then load. Ordered so the loader never
/// mmaps a file the migration is about to move.
pub fn spawn_migration_and_model_loader(app: AppHandle, settings: AiSettings) {
	std::thread::spawn(move || {
		if let Some(state) = app.try_state::<AppState>() {
			models::migrate_legacy_models(&state.models_dir(), &models::primary_hub_cache());
		}
		run_model_loader(app, settings);
	});
}

/// What the model loader does with the whisper slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhisperPlan {
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
enum SttStatusPlan {
	/// the external endpoint transcribes
	External,
	/// Apple Speech transcribes
	AppleReady,
	/// explicit Apple on a system without it: whisper stands in, and the
	/// status says why
	AppleUnsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LlmPlan {
	External,
	Load(&'static str),
	Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LoadPlan {
	whisper: WhisperPlan,
	stt_status: Option<SttStatusPlan>,
	llm: LlmPlan,
}

/// The loader's whole decision, pure: which engines to load or unload
/// and which status to report, given the settings, whether Apple Speech
/// exists here, and which catalog models are downloaded.
fn plan_model_load(
	settings: &AiSettings,
	apple_available: bool,
	downloaded: impl Fn(&models::ModelSpec) -> bool,
) -> LoadPlan {
	use models::SpeechEngine;
	let whisper_model = match models::find_model(&settings.stt_model, ModelKind::Stt) {
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
		match models::find_model(&settings.llm_model, ModelKind::Llm) {
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

fn run_model_loader(app: AppHandle, settings: AiSettings) {
	let Some(state) = app.try_state::<AppState>() else {
		return;
	};
	let plan = plan_model_load(&settings, apple::speech_available(), |spec| {
		state.is_model_downloaded(spec)
	});
	// Each slot change first waits for an in-flight load of that engine
	// (an activation, a download's auto-load) and holds the slot while
	// unloading: an unload racing a running load would be undone when
	// that load installs its engine, contradicting the reported status.
	const SLOT_WAIT: std::time::Duration = std::time::Duration::from_secs(300);

	let stt_slot = models::EngineSlotClaim::acquire(&state.stt_loading, SLOT_WAIT);
	match plan.whisper {
		WhisperPlan::Load(id) => {
			// load_stt claims the slot itself
			stt_slot.release();
			if let Some(spec) = models::find_model(id, ModelKind::Stt) {
				if let Err(e) = state.load_stt(&app, spec) {
					log::error!("startup STT load failed: {e}");
				}
			}
		}
		WhisperPlan::Missing => {
			state.runtime.lock().unwrap_or_else(|e| e.into_inner()).stt = None;
			*state.stt_status.lock().unwrap_or_else(|e| e.into_inner()) =
				models::EngineStatus::missing();
			state.emit_stt_status(&app);
		}
		WhisperPlan::Unload => {
			state.runtime.lock().unwrap_or_else(|e| e.into_inner()).stt = None;
		}
	}
	if let Some(status) = plan.stt_status {
		*state.stt_status.lock().unwrap_or_else(|e| e.into_inner()) = match status {
			SttStatusPlan::External => models::EngineStatus::external(),
			SttStatusPlan::AppleReady => models::EngineStatus::ready(Some("apple-speech")),
			SttStatusPlan::AppleUnsupported => models::EngineStatus::error(
				Some("apple-speech"),
				"Apple Speech requires macOS 26+ - using whisper instead",
			),
		};
		state.emit_stt_status(&app);
	}

	let llm_slot = models::EngineSlotClaim::acquire(&state.llm_loading, SLOT_WAIT);
	match plan.llm {
		LlmPlan::Load(id) => {
			// load_llm claims the slot itself
			llm_slot.release();
			if let Some(spec) = models::find_model(id, ModelKind::Llm) {
				if let Err(e) = state.load_llm(&app, spec) {
					log::error!("startup LLM load failed: {e}");
				}
			}
		}
		LlmPlan::External | LlmPlan::Missing => {
			state.runtime.lock().unwrap_or_else(|e| e.into_inner()).llm = None;
			*state.llm_status.lock().unwrap_or_else(|e| e.into_inner()) =
				if plan.llm == LlmPlan::External {
					models::EngineStatus::external()
				} else {
					models::EngineStatus::missing()
				};
			state.emit_llm_status(&app);
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
