mod commands;
mod db;
pub mod llm;
pub mod models;
pub mod prompts;
mod reminders;
pub mod stt;
pub mod types;

use chrono::Utc;
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};

use models::{AiSettings, AppState, ModelKind};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	tauri::Builder::default()
		.plugin(tauri_plugin_notification::init())
		.plugin(tauri_plugin_dialog::init())
		.invoke_handler(tauri::generate_handler![
			commands::data::get_user,
			commands::data::get_user_trial,
			commands::data::get_daily_status,
			commands::data::get_daily_list,
			commands::data::get_accountability,
			commands::data::get_all_ideas,
			commands::data::get_idea,
			commands::data::get_idea_children,
			commands::data::create_idea,
			commands::data::update_idea,
			commands::data::mark_idea_read,
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
			commands::settings::get_user_settings,
			commands::settings::save_user_settings,
			commands::settings::get_ai_settings,
			commands::settings::save_ai_settings,
			commands::settings::test_llm_endpoint,
			commands::settings::test_stt_endpoint,
			commands::models_cmd::list_models,
			commands::models_cmd::get_runtime_status,
			commands::models_cmd::download_model,
			commands::models_cmd::delete_model,
			commands::models_cmd::activate_model,
			commands::share::export_idea,
			commands::share::import_share,
		])
		.setup(|app| {
			let data_dir = app
				.path()
				.app_data_dir()
				.expect("failed to resolve app data dir");
			std::fs::create_dir_all(data_dir.join("models")).ok();

			let db = db::Db::open(&data_dir.join("brainstory.db"))
				.expect("failed to open database");
			if db.get_setting("created_at").is_none() {
				let now = Utc::now().naive_utc().format("%Y-%m-%dT%H:%M:%SZ").to_string();
				db.set_setting("created_at", &now);
			}

			app.manage(AppState::new(db, data_dir));

			setup_tray(app)?;
			reminders::spawn(app.handle().clone());
			spawn_model_loader(app.handle().clone(), AiSettings::load(&app.state::<AppState>().db));

			Ok(())
		})
		.on_window_event(|window, event| {
			// Closing the window hides it to the tray so daily reminders
			// keep working; quit via the tray menu.
			if let WindowEvent::CloseRequested { api, .. } = event {
				window.hide().ok();
				api.prevent_close();
			}
		})
		.run(tauri::generate_context!())
		.expect("error while running brainstory");
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
	let reminder_enabled = app
		.state::<AppState>()
		.db
		.get_setting("reminder_enabled")
		.map(|v| v == "true")
		.unwrap_or(false);

	let open = MenuItem::with_id(app, "open", "Open Brainstory", true, None::<&str>)?;
	let reminder =
		CheckMenuItem::with_id(app, "reminder", "Daily reminder", true, reminder_enabled, None::<&str>)?;
	let quit = MenuItem::with_id(app, "quit", "Quit Brainstory", true, None::<&str>)?;
	let menu = Menu::with_items(app, &[&open, &reminder, &quit])?;

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
				state.db.set_setting("reminder_enabled", if enabled { "true" } else { "false" });
			}
			"quit" => {
				app.exit(0);
			}
			_ => {}
		})
		.on_tray_icon_event(|tray, event| {
			if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
				show_main_window(tray.app_handle());
			}
		});

	if let Some(icon) = app.default_window_icon() {
		tray = tray.icon(icon.clone());
	}
	tray.build(app)?;
	Ok(())
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
		let Some(state) = app.try_state::<AppState>() else { return };

		// STT: load local whisper unless an external endpoint is configured.
		if settings.ext_stt_base_url.is_empty() {
			if let Some(spec) = models::find_model(&settings.stt_model, ModelKind::Stt) {
				if state.is_model_downloaded(spec) {
					state.load_stt(&app, spec);
				}
			}
		} else {
			let mut s = state.stt_status.lock().unwrap();
			*s = models::EngineStatus::new("external", None, None);
		}

		// LLM: load local model unless external mode is active.
		if settings.llm_mode != "local" {
			let mut s = state.llm_status.lock().unwrap();
			*s = models::EngineStatus::new("external", None, None);
			return;
		}
		if let Some(spec) = models::find_model(&settings.llm_model, ModelKind::Llm) {
			if state.is_model_downloaded(spec) {
				state.load_llm(&app, spec);
			}
		}
	});
}
