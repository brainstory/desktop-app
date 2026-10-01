use std::time::Duration;

use chrono::Timelike;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::keys;
use crate::models::AppState;

/// Wakes the reminder loop early when reminder settings change (so a new
/// time takes effect immediately instead of at the next scheduled wake).
pub static REMINDER_SETTINGS_CHANGED: std::sync::LazyLock<tokio::sync::Notify> =
	std::sync::LazyLock::new(tokio::sync::Notify::new);

/// The loop's maximum sleep even when nothing is due: settings can be
/// edited outside the app's control (hand-edited rows), and the clock
/// itself can jump (sleep/wake across days).
const MAX_SLEEP: Duration = Duration::from_secs(15 * 60);

/// Reminder scheduler. Computes the next due instant and sleeps until it
/// (waking early when reminder settings change), instead of polling every
/// 20 s forever. Fires the notification once per day at (or after) the
/// configured time while the app is running (kept alive by the tray icon).
pub fn spawn(app: AppHandle) {
	tauri::async_runtime::spawn(async move {
		loop {
			let sleep_for = tick(&app).await;
			// Wake on a settings change or the computed deadline, whichever
			// comes first - but never sleep unboundedly.
			tokio::select! {
				_ = REMINDER_SETTINGS_CHANGED.notified() => {}
				_ = tokio::time::sleep(sleep_for) => {}
			}
		}
	});
}

/// One scheduler pass. Returns how long to sleep before the next pass.
async fn tick(app: &AppHandle) -> Duration {
	let Some(state) = app.try_state::<AppState>() else {
		return MAX_SLEEP;
	};

	let enabled = state
		.db
		.get_setting(keys::setting::REMINDER_ENABLED)
		.map(|v| v == "true")
		.unwrap_or(false);
	if !enabled {
		return MAX_SLEEP;
	}

	let time_str = state
		.db
		.get_setting(keys::setting::REMINDER_TIME)
		.filter(|s| !s.is_empty())
		.unwrap_or_else(|| "09:00".into());
	let (hour, minute) = parse_time(&time_str);
	let now = chrono::Local::now();
	let today = now.format("%Y-%m-%d").to_string();
	let last_fired = state
		.db
		.get_setting(keys::setting::REMINDER_LAST_FIRED)
		.unwrap_or_default();

	if last_fired == today {
		// done for today: sleep until the reminder time tomorrow
		return until_next_due(now, hour, minute).min(MAX_SLEEP);
	}

	let due = now.hour() > hour || (now.hour() == hour && now.minute() >= minute);
	if !due {
		return until_next_due(now, hour, minute).min(MAX_SLEEP);
	}

	// Already brainstormed today? Then the reminder has nothing to do
	// (record it as handled so the loop sleeps until tomorrow).
	if state.db.has_activity_today() {
		if let Err(e) = state
			.db
			.set_setting(keys::setting::REMINDER_LAST_FIRED, &today)
		{
			log::warn!("failed to record reminder: {e}");
		}
		return until_next_due(now, hour, minute).min(MAX_SLEEP);
	}

	let _ = app
		.notification()
		.builder()
		.title("Brainstory")
		.body("What's on your mind today? Take a few minutes to think out loud.")
		.show();
	if let Err(e) = state
		.db
		.set_setting(keys::setting::REMINDER_LAST_FIRED, &today)
	{
		log::warn!("failed to record reminder: {e}");
	}
	until_next_due(chrono::Local::now(), hour, minute).min(MAX_SLEEP)
}

/// How long until the reminder is due again: the next occurrence of
/// HH:MM local time (tomorrow's if today's already passed). The pure
/// instant computation is split out for tests.
fn until_next_due(now: chrono::DateTime<chrono::Local>, hour: u32, minute: u32) -> Duration {
	use chrono::TimeZone;
	let today_due = now
		.date_naive()
		.and_hms_opt(hour.min(23), minute.min(59), 0)
		.expect("clamped hour/minute are always valid");
	let today_due = chrono::Local
		.from_local_datetime(&today_due)
		.single()
		.unwrap_or(now);
	let next = if today_due > now {
		today_due
	} else {
		today_due + chrono::Duration::days(1)
	};
	(next - now).to_std().unwrap_or(MAX_SLEEP)
}

fn parse_time(time: &str) -> (u32, u32) {
	// Anything invalid falls back to the same default a fresh install
	// uses; the settings form only persists strict HH:MM values.
	crate::commands::settings::parse_hhmm(time).unwrap_or((9, 0))
}

#[cfg(test)]
mod tests {
	use chrono::TimeZone;

	use super::{parse_time, until_next_due};
	use crate::commands::settings::parse_hhmm;
	use std::time::Duration;

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

	#[test]
	fn next_due_is_today_before_the_time_and_tomorrow_after() {
		// 08:00 with a 09:00 reminder -> due in an hour
		let morning = chrono::Local
			.with_ymd_and_hms(2026, 9, 30, 8, 0, 0)
			.unwrap();
		let until = until_next_due(morning, 9, 0);
		assert!(
			until >= Duration::from_secs(59 * 60) && until <= Duration::from_secs(61 * 60),
			"~1h until due, got {until:?}"
		);

		// 10:00 with a 09:00 reminder -> due tomorrow 09:00 (23h)
		let after = chrono::Local
			.with_ymd_and_hms(2026, 9, 30, 10, 0, 0)
			.unwrap();
		let until = until_next_due(after, 9, 0);
		assert!(
			until >= Duration::from_secs(23 * 3600 - 60)
				&& until <= Duration::from_secs(23 * 3600 + 60),
			"~23h until tomorrow's reminder, got {until:?}"
		);

		// exactly on the minute counts as due already (>= comparison)
		let exact = chrono::Local
			.with_ymd_and_hms(2026, 9, 30, 9, 0, 0)
			.unwrap();
		let until = until_next_due(exact, 9, 0);
		assert_eq!(until.as_secs(), 24 * 3600, "due now -> tomorrow");
	}

	#[test]
	fn raw_next_due_is_never_capped() {
		// 23h59m until tomorrow's reminder is returned uncapped; the
		// loop applies its own 15-minute cap on top
		let late = chrono::Local
			.with_ymd_and_hms(2026, 9, 30, 23, 59, 0)
			.unwrap();
		assert_eq!(until_next_due(late, 23, 58).as_secs(), 24 * 3600 - 60);
	}
}
