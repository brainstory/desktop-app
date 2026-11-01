//! The app shell around the windows and pages: the tray menu, dock and
//! tray presence, quick capture, and the quit path. Split out of lib.rs
//! so each piece stays small enough for rustfmt to keep formatting it.

use std::sync::Mutex;

use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::models::AppState;
use crate::{keys, reminders};

/// Handle to the tray's reminder check item, kept so settings-page changes
/// can sync its checkmark (otherwise it shows the opposite of reality).
static TRAY_REMINDER_ITEM: Mutex<Option<CheckMenuItem<Wry>>> = Mutex::new(None);

pub(crate) fn sync_tray_reminder_check(enabled: bool) {
	let guard = TRAY_REMINDER_ITEM.lock().unwrap_or_else(|e| e.into_inner());
	if let Some(item) = guard.as_ref() {
		if let Err(e) = item.set_checked(enabled) {
			log::warn!("failed to update tray reminder checkmark: {e}");
		}
	}
}

pub(crate) fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
	let reminder_enabled = app
		.state::<AppState>()
		.db
		.get_setting(keys::setting::REMINDER_ENABLED)
		.map(|v| v == "true")
		.unwrap_or(false);

	let new_idea = MenuItem::with_id(app, "new_idea", "New Idea", true, None::<&str>)?;
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
	let menu = Menu::with_items(app, &[&new_idea, &open, &reminder, &quit])?;

	*TRAY_REMINDER_ITEM.lock().unwrap_or_else(|e| e.into_inner()) = Some(reminder);

	let mut tray = TrayIconBuilder::with_id("main-tray")
		.menu(&menu)
		.show_menu_on_left_click(false)
		.tooltip("Brainstory")
		.on_menu_event(|app, event| match event.id().as_ref() {
			"new_idea" => open_new_idea(app),
			"open" => show_main_window(app),
			"reminder" => {
				let app = app.clone();
				// the DB work stays off the main thread (the menu event
				// runs there); the flip itself is one transaction
				tauri::async_runtime::spawn_blocking(move || {
					let state = app.state::<AppState>();
					let enabled = match state
						.db
						.toggle_setting_bool(keys::setting::REMINDER_ENABLED, false)
					{
						Ok(enabled) => enabled,
						Err(e) => {
							log::error!("failed to save reminder setting: {e}");
							return;
						}
					};
					// enabling after today's time must not fire at once:
					// mark today as handled before waking the scheduler
					if enabled {
						if let Some(today) =
							reminders::initial_last_fired_for_enable(&state.db, None)
						{
							if let Err(e) = state
								.db
								.set_setting(keys::setting::REMINDER_LAST_FIRED, &today)
							{
								log::warn!("failed to record reminder state: {e}");
							}
						}
					}
					// keep the native checkmark and the setting in lockstep
					sync_tray_reminder_check(enabled);
					// the settings page caches get_user_settings: tell it
					// the reminder moved so its toggle follows
					if let Err(e) = app.emit(
						"user-settings-changed",
						serde_json::json!({ "reminderEnabled": enabled }),
					) {
						log::warn!("failed to broadcast the reminder change: {e}");
					}
					// wake the scheduler so the toggle applies immediately
					reminders::notify_settings_changed();
				});
			}
			"quit" => {
				shutdown_and_exit(app);
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
pub(crate) fn apply_presence(app: &AppHandle, dock: bool, tray: bool) {
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
pub(crate) fn shutdown_and_exit(app: &AppHandle) -> ! {
	let state = app.state::<AppState>();
	// new writes refuse to start from here on; in-flight ones are
	// waited out below instead of being cut off mid-transaction
	state
		.shutting_down
		.store(true, std::sync::atomic::Ordering::SeqCst);
	// The hidden-in-tray close path destroys the window without the
	// page ever running its pagehide flush: ask for it explicitly and
	// give the (already async) save a moment to land.
	if state
		.quit_on_close
		.load(std::sync::atomic::Ordering::Relaxed)
	{
		if let Some(window) = app.get_webview_window("main") {
			window
				.eval("window.dispatchEvent(new Event('pagehide'));")
				.ok();
		}
		std::thread::sleep(std::time::Duration::from_secs(1));
	}
	// wait out in-flight writes, then leave a fully checkpointed file
	state.db.quiesce(std::time::Duration::from_secs(2));
	log::logger().flush();
	// _exit stays: the vendored ggml static destructors abort if the
	// process unwinds normally
	unsafe { libc::_exit(0) }
}

fn show_main_window(app: &AppHandle) {
	if let Some(window) = app.get_webview_window("main") {
		window.show().ok();
		window.unminimize().ok();
		window.set_focus().ok();
	}
}

/// Where quick capture lands: a brand-new idea session.
pub(crate) const QUICK_CAPTURE_SHORTCUT: &str = "CmdOrCtrl+Shift+Space";

/// Open the window on a fresh idea page. Wired to the tray's "New Idea"
/// item and the global quick capture shortcut.
pub(crate) fn open_new_idea(app: &AppHandle) {
	show_main_window(app);
	// An in-progress recording or generation must not be destroyed by
	// the page load (a dead IPC channel cancels the generation): bring
	// the window up and let the page explain instead of navigating.
	let generating = app
		.state::<AppState>()
		.active_generations
		.load(std::sync::atomic::Ordering::SeqCst)
		> 0;
	if crate::voice::is_capturing() || generating {
		if let Err(e) = app.emit("quick-capture-blocked", ()) {
			log::warn!("failed to announce the blocked quick capture: {e}");
		}
		return;
	}
	let Some(window) = app.get_webview_window("main") else {
		return;
	};
	// Already on a fresh chat? The navigation would only remount the
	// page (dropping, say, an unsent typed draft) for nothing.
	let on_fresh_chat = window
		.url()
		.is_ok_and(|url| url.path() == "/chat" && !url.query_pairs().any(|(k, _)| k == "id"));
	if on_fresh_chat {
		return;
	}
	// A full navigation, like every other page change in the app (Astro
	// pages are separate documents); an in-progress draft is flushed to
	// the database by the page's own pagehide hook.
	if let Err(e) = window.eval("window.location.href = '/chat';") {
		log::warn!("failed to open the new idea page: {e}");
	}
}

/// The quick capture shortcut is on unless the setting says otherwise;
/// the setting exists so a conflicting global hotkey can be released for
/// another app (and retried later).
pub(crate) fn quick_capture_enabled(db: &crate::db::Db) -> bool {
	db.get_setting(crate::keys::setting::QUICK_CAPTURE_ENABLED)
		.map(|v| v == "true")
		.unwrap_or(true)
}

/// Register the global quick capture shortcut. Failure is not fatal: the
/// combination may be owned by another app, and the tray's "New Idea"
/// item keeps working, so only a warning is logged.
pub(crate) fn register_quick_capture_shortcut(app: &AppHandle) {
	use tauri_plugin_global_shortcut::GlobalShortcutExt;
	if let Err(e) = app.global_shortcut().register(QUICK_CAPTURE_SHORTCUT) {
		log::warn!("could not register the quick capture shortcut {QUICK_CAPTURE_SHORTCUT}: {e}");
	}
}
