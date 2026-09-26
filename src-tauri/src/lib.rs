pub mod apple;
mod commands;
pub mod db;
pub mod llm;
pub mod models;
pub mod prompts;
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
			commands::settings::get_ai_settings,
			commands::settings::save_ai_settings,
			commands::settings::test_llm_endpoint,
			commands::settings::test_stt_endpoint,
			commands::models_cmd::list_models,
			commands::models_cmd::get_runtime_status,
			commands::models_cmd::get_apple_stt_status,
			commands::models_cmd::download_model,
			commands::models_cmd::cancel_download,
			commands::models_cmd::delete_model,
			commands::models_cmd::activate_model,
			commands::share::export_idea,
			commands::share::import_share,
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
			// Surface webview JS errors in the window title so a crashed
			// page is diagnosable from the log instead of being just a
			// white rectangle.
			let _ = webview.eval(
				"window.addEventListener('error', function(e){ document.title = 'JSERR: ' + e.message; });\n\
				 window.addEventListener('unhandledrejection', function(e){ document.title = 'PROMISE-REJ: ' + e.reason; });",
			);
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
			sweep_stale_part_files(&data_dir.join("models"));

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
			if db.get_setting("created_at").is_none() {
				// naive UTC, no trailing Z (the frontend appends it itself)
				let now = Utc::now()
					.naive_utc()
					.format("%Y-%m-%dT%H:%M:%S")
					.to_string();
				if let Err(e) = db.set_setting("created_at", &now) {
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
					.get_setting("show_in_dock")
					.map(|v| v == "true")
					.unwrap_or(true);
				let tray = state
					.db
					.get_setting("show_in_tray")
					.map(|v| v == "true")
					.unwrap_or(true);
				apply_presence(app.handle(), dock, tray);
			}

			reminders::spawn(app.handle().clone());

			// Temporary startup diagnostic: after the onboarding redirect
			// settles, a JS error would have renamed the window title
			// (see the on_page_load error hook).
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
			spawn_model_loader(
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

/// Remove `.part` files left behind by a quit (or crash) mid-download; the
/// in-process error path can't clean up when the process itself is gone.
fn sweep_stale_part_files(models_dir: &std::path::Path) {
	let Ok(entries) = std::fs::read_dir(models_dir) else {
		return;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		let is_part = path.extension().map(|e| e == "part").unwrap_or(false);
		if is_part {
			log::warn!("removing leftover partial download {}", path.display());
			if let Err(e) = std::fs::remove_file(&path) {
				log::warn!("could not remove {}: {e}", path.display());
			}
		}
	}
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
		Err(e) if is_db_corruption(&e) => {
			log::error!("database is corrupt ({e}); quarantining it and starting fresh");
			let stamp = Utc::now().format("%Y%m%d-%H%M%S");
			let corrupt = data_dir.join(format!("brainstory.db.corrupt-{stamp}"));
			if let Err(rename_err) = std::fs::rename(&db_path, &corrupt) {
				log::error!("failed to quarantine the corrupt database: {rename_err}");
			}
			// move WAL sidecars along with it so the fresh DB starts clean
			for ext in ["wal", "shm"] {
				let _ = std::fs::rename(db_path.with_extension(ext), corrupt.with_extension(ext));
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
		.get_setting("reminder_enabled")
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
					.get_setting("reminder_enabled")
					.map(|v| v == "true")
					.unwrap_or(false);
				if let Err(e) = state
					.db
					.set_setting("reminder_enabled", if enabled { "true" } else { "false" })
				{
					log::error!("failed to save reminder setting: {e}");
				}
				// keep the native checkmark and the setting in lockstep
				sync_tray_reminder_check(enabled);
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
			let mtm =
				objc2::MainThreadMarker::new().expect("apply_presence must run on the main thread");
			objc2_app_kit::NSApplication::sharedApplication(mtm).activate();
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
/// skipping finalization is safe.
pub fn force_exit() -> ! {
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
	std::thread::spawn(move || {
		let Some(state) = app.try_state::<AppState>() else {
			return;
		};

		// STT: external endpoint wins; otherwise the engine setting picks
		// Apple Speech (macOS 26+, zero downloads) or local whisper.
		if settings.ext_stt_base_url.is_empty() {
			let load_whisper = |state: &AppState, app: &AppHandle| {
				if let Some(spec) = models::find_model(&settings.stt_model, ModelKind::Stt) {
					if state.is_model_downloaded(spec) {
						if let Err(e) = state.load_stt(app, spec) {
							log::error!("startup STT load failed: {e}");
						}
					}
				}
			};
			if settings.stt_engine == "apple" && !apple::speech_available() {
				// Explicit Apple on an unsupported system: degrade to
				// whisper but say why, instead of silently ignoring it.
				load_whisper(&state, &app);
				let mut s = state.stt_status.lock().unwrap_or_else(|e| e.into_inner());
				*s = models::EngineStatus::new(
					"error",
					Some("apple-speech"),
					Some("Apple Speech requires macOS 26+ - using whisper instead"),
				);
			} else {
				match settings.effective_stt_engine() {
					"apple" => {
						if settings.stt_engine == "auto" {
							// auto: keep a downloaded whisper model hot as
							// the fallback behind the Apple engine.
							load_whisper(&state, &app);
						} else {
							// explicit apple: whisper is not needed at all;
							// free its memory.
							state.runtime.lock().unwrap_or_else(|e| e.into_inner()).stt = None;
						}
						let mut s = state.stt_status.lock().unwrap_or_else(|e| e.into_inner());
						*s = models::EngineStatus::new("ready", Some("apple-speech"), None);
					}
					_ => load_whisper(&state, &app),
				}
			}
		} else {
			let mut s = state.stt_status.lock().unwrap_or_else(|e| e.into_inner());
			*s = models::EngineStatus::new("external", None, None);
		}

		// LLM: load local model unless external mode is active.
		if settings.llm_mode != "local" {
			let mut s = state.llm_status.lock().unwrap_or_else(|e| e.into_inner());
			*s = models::EngineStatus::new("external", None, None);
			return;
		}
		if let Some(spec) = models::find_model(&settings.llm_model, ModelKind::Llm) {
			if state.is_model_downloaded(spec) {
				if let Err(e) = state.load_llm(&app, spec) {
					log::error!("startup LLM load failed: {e}");
				}
			}
		}
	});
}
