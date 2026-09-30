use std::time::Duration;

use chrono::Timelike;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::models::AppState;

/// Reminder scheduler. Reads the reminder settings periodically and fires a
/// system notification once per day at (or after) the configured time while
/// the app is running (kept alive by the tray icon).
pub fn spawn(app: AppHandle) {
	tauri::async_runtime::spawn(async move {
		loop {
			tokio::time::sleep(Duration::from_secs(20)).await;
			let Some(state) = app.try_state::<AppState>() else {
				continue;
			};

			let enabled = state
				.db
				.get_setting("reminder_enabled")
				.map(|v| v == "true")
				.unwrap_or(false);
			if !enabled {
				continue;
			}

			let time_str = state
				.db
				.get_setting("reminder_time")
				.filter(|s| !s.is_empty())
				.unwrap_or_else(|| "09:00".into());
			let (hour, minute) = parse_time(&time_str);
			let now = chrono::Local::now();
			let today = now.format("%Y-%m-%d").to_string();
			let last_fired = state
				.db
				.get_setting("reminder_last_fired")
				.unwrap_or_default();
			if last_fired == today {
				continue;
			}

			let due = now.hour() > hour || (now.hour() == hour && now.minute() >= minute);
			if !due {
				continue;
			}

			// Already brainstormed today? Then the reminder has nothing to do.
			if state.db.has_activity_today() {
				if let Err(e) = state.db.set_setting("reminder_last_fired", &today) {
					log::warn!("failed to record reminder: {e}");
				}
				continue;
			}

			let _ = app
				.notification()
				.builder()
				.title("Brainstory")
				.body("What's on your mind today? Take a few minutes to think out loud.")
				.show();
			if let Err(e) = state.db.set_setting("reminder_last_fired", &today) {
				log::warn!("failed to record reminder: {e}");
			}
		}
	});
}

fn parse_time(time: &str) -> (u32, u32) {
	// Anything invalid falls back to the same default a fresh install
	// uses; the settings form only persists strict HH:MM values.
	crate::commands::settings::parse_hhmm(time).unwrap_or((9, 0))
}

#[cfg(test)]
mod tests {
	use super::parse_time;
	use crate::commands::settings::parse_hhmm;

	/// The reminder loop and the settings form must agree on what a valid
	/// time looks like: parse_time accepts exactly what parse_hhmm (the
	/// form's validator) accepts, and falls back otherwise.
	#[test]
	fn parse_time_matches_valid_reminder_time() {
		let corpus = [
			"09:00", "9:00", "00:00", "23:59", "24:00", "12:5", "12:60", "", "noon", "12:00:00",
		];
		for raw in corpus {
			match parse_hhmm(raw) {
				Some(parsed) => assert_eq!(parse_time(raw), parsed, "divergence on {raw:?}"),
				None => assert_eq!(parse_time(raw), (9, 0), "divergence on {raw:?}"),
			}
		}
	}
}
