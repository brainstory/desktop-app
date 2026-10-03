use std::time::Duration;

use chrono::{DateTime, LocalResult, NaiveDateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::keys;
use crate::models::AppState;

/// Wakes the reminder loop early when reminder settings change (so a new
/// time takes effect immediately instead of at the next scheduled wake).
static REMINDER_SETTINGS_CHANGED: std::sync::LazyLock<tokio::sync::Notify> =
	std::sync::LazyLock::new(tokio::sync::Notify::new);

/// Wake the reminder loop because its settings changed. notify_one, not
/// notify_waiters: the latter only wakes a task already waiting, so a
/// change landing while the loop runs tick() was lost and the loop slept
/// until its old deadline (up to 15 minutes). notify_one stores a permit
/// that the next wait consumes immediately; the single loop is the only
/// waiter.
pub fn notify_settings_changed() {
	REMINDER_SETTINGS_CHANGED.notify_one();
}

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
	// The reminder clock runs in the SAME zone as the daily boundaries:
	// the stored user timezone when one resolves, the OS zone otherwise
	// (Db::active_zone mirrors the stored row; the reminder's "today"
	// and has_activity_today's "today" can never disagree).
	tick_in_zone(app, &state, state.db.active_zone()).await
}

/// The scheduler pass for one active zone (None = OS-local).
async fn tick_in_zone(app: &AppHandle, state: &AppState, zone: Option<Tz>) -> Duration {
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
	let now = Utc::now();
	let today = wall_clock(zone, &now).format("%Y-%m-%d").to_string();
	let last_fired = state
		.db
		.get_setting(keys::setting::REMINDER_LAST_FIRED)
		.unwrap_or_default();

	if last_fired == today {
		// done for today: sleep until the reminder time tomorrow
		return until_next_due(zone, now, hour, minute).min(MAX_SLEEP);
	}

	if !is_due(wall_clock(zone, &now).time(), hour, minute) {
		return until_next_due(zone, now, hour, minute).min(MAX_SLEEP);
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
		return until_next_due(zone, now, hour, minute).min(MAX_SLEEP);
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
	until_next_due(zone, Utc::now(), hour, minute).min(MAX_SLEEP)
}

/// True once today's reminder time (HH:MM in the active zone) has been
/// reached; the minute itself counts as due.
fn is_due(now: chrono::NaiveTime, hour: u32, minute: u32) -> bool {
	(now.hour(), now.minute()) >= (hour, minute)
}

/// The wall-clock time an instant reads as in the active zone.
fn wall_clock(zone: Option<Tz>, now: &DateTime<Utc>) -> NaiveDateTime {
	match zone {
		Some(tz) => now.with_timezone(&tz).naive_local(),
		None => now.with_timezone(&chrono::Local).naive_local(),
	}
}

/// Resolve one wall-clock time in `zone` to an instant, under the DST
/// policy:
/// - unambiguous -> that instant;
/// - ambiguous (a fold, where the wall time happens twice) -> the
///   EARLIEST instant, so the reminder fires at the first chance; it
///   fires at most once per calendar day anyway (REMINDER_LAST_FIRED
///   records the zone-local date), so it never double-fires;
/// - nonexistent (a gap, where the clock jumps past the wall time) ->
///   the earliest valid instant AFTER the gap, found by stepping
///   forward a minute at a time (real-world gaps are under two hours),
///   so the reminder is never skipped to the next day.
fn resolve_wall_time<Z: TimeZone>(zone: &Z, local: NaiveDateTime) -> Option<DateTime<Utc>> {
	match zone.from_local_datetime(&local) {
		LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
		LocalResult::Ambiguous(earliest, _) => Some(earliest.with_timezone(&Utc)),
		LocalResult::None => {
			let mut probe = local;
			for _ in 0..(6 * 60) {
				probe += chrono::Duration::minutes(1);
				match zone.from_local_datetime(&probe) {
					LocalResult::Single(dt) => return Some(dt.with_timezone(&Utc)),
					LocalResult::Ambiguous(earliest, _) => {
						return Some(earliest.with_timezone(&Utc))
					}
					LocalResult::None => {}
				}
			}
			None
		}
	}
}

fn local_to_utc(zone: Option<Tz>, local: NaiveDateTime) -> Option<DateTime<Utc>> {
	match zone {
		Some(tz) => resolve_wall_time(&tz, local),
		None => resolve_wall_time(&chrono::Local, local),
	}
}

/// How long until the reminder is due again: the next occurrence of
/// HH:MM in the active zone (tomorrow's if today's already passed).
/// Tomorrow is resolved on tomorrow's calendar date in that zone, so a
/// DST shift between the days cannot bend the wall-clock time. The
/// pure instant computation is split out for tests.
fn until_next_due(zone: Option<Tz>, now: DateTime<Utc>, hour: u32, minute: u32) -> Duration {
	let today_wall = wall_clock(zone, &now)
		.date()
		.and_hms_opt(hour.min(23), minute.min(59), 0)
		.expect("clamped hour/minute are always valid");
	let today_due = local_to_utc(zone, today_wall);
	// tomorrow's wall-clock time; computed eagerly, resolution is cheap
	let tomorrow_due = local_to_utc(zone, today_wall + chrono::Duration::days(1));
	let next = match (today_due, tomorrow_due) {
		(Some(today), _) if today > now => today,
		(_, Some(tomorrow)) => tomorrow,
		// pathological zones where neither resolves: keep the old
		// 24h-later behavior so the loop always has a deadline
		_ => now + chrono::Duration::days(1),
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
	use chrono_tz::Tz;
	use std::time::Duration;

	/// A wall time in the runner's OS zone, as a UTC instant (the zone
	/// the previous OS-local behavior ran in).
	fn os_local_wall(y: i32, m: u32, d: i32, h: u32, min: u32) -> chrono::DateTime<chrono::Utc> {
		chrono::Local
			.with_ymd_and_hms(y, m, d as u32, h, min, 0)
			.unwrap()
			.with_timezone(&chrono::Utc)
	}

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
		let morning = os_local_wall(2026, 9, 30, 8, 0);
		let until = until_next_due(None, morning, 9, 0);
		assert!(
			until >= Duration::from_secs(59 * 60) && until <= Duration::from_secs(61 * 60),
			"~1h until due, got {until:?}"
		);

		// 10:00 with a 09:00 reminder -> due tomorrow 09:00 (23h)
		let after = os_local_wall(2026, 9, 30, 10, 0);
		let until = until_next_due(None, after, 9, 0);
		assert!(
			until >= Duration::from_secs(23 * 3600 - 60)
				&& until <= Duration::from_secs(23 * 3600 + 60),
			"~23h until tomorrow's reminder, got {until:?}"
		);

		// exactly on the minute counts as due already (>= comparison)
		let exact = os_local_wall(2026, 9, 30, 9, 0);
		let until = until_next_due(None, exact, 9, 0);
		assert_eq!(until.as_secs(), 24 * 3600, "due now -> tomorrow");
	}

	#[test]
	fn next_due_runs_in_the_stored_zone_not_the_os_zone() {
		// 2026-09-30 20:00 UTC is 2026-10-01 05:00 in Tokyo (UTC+9):
		// a 09:00 Tokyo reminder is due today-in-Tokyo at 09:00 JST
		// (= 00:00 UTC), four hours away. Under the OS zone of a
		// western runner it would be ~17h away instead.
		let tokyo: Option<Tz> = Some(chrono_tz::Asia::Tokyo);
		let now = chrono::Utc.with_ymd_and_hms(2026, 9, 30, 20, 0, 0).unwrap();
		assert_eq!(until_next_due(tokyo, now, 9, 0).as_secs(), 4 * 3600);
		// the same instant under a UTC-4 zone: 16:00 local, due tomorrow
		// 09:00 local = 13:00 UTC, 17h away
		let ny: Option<Tz> = Some(chrono_tz::America::New_York);
		assert_eq!(until_next_due(ny, now, 9, 0).as_secs(), 17 * 3600);
	}

	#[test]
	fn next_due_in_a_gap_time_fires_at_the_earliest_instant_after_the_gap() {
		// America/New_York springs forward on 2027-03-14: 02:00 jumps to
		// 03:00, so a 02:30 reminder time does not exist. At 01:00 EST
		// (= 06:00 UTC) the next due instant is 03:00 EDT (= 07:00 UTC):
		// one real hour later, never skipped to the next day.
		let ny: Option<Tz> = Some(chrono_tz::America::New_York);
		let now = chrono::Utc.with_ymd_and_hms(2027, 3, 14, 6, 0, 0).unwrap();
		let until = until_next_due(ny, now, 2, 30);
		assert_eq!(
			until.as_secs(),
			3600,
			"gap -> earliest instant after the gap"
		);
	}

	#[test]
	fn next_due_in_a_fold_time_fires_once_at_the_earliest_instant() {
		// America/New_York falls back on 2027-11-07: 02:00 jumps back to
		// 01:00, so 01:30 happens twice (01:30 EDT and 01:30 EST). At
		// 00:30 EDT (= 04:30 UTC) the next due instant is the FIRST
		// 01:30 (EDT, = 05:30 UTC): one hour away, one instant only -
		// the once-per-day bookkeeping keeps it from firing twice.
		let ny: Option<Tz> = Some(chrono_tz::America::New_York);
		let now = chrono::Utc.with_ymd_and_hms(2027, 11, 7, 4, 30, 0).unwrap();
		let until = until_next_due(ny, now, 1, 30);
		assert_eq!(
			until.as_secs(),
			3600,
			"fold -> earliest of the two instants"
		);
	}

	#[test]
	fn the_reminder_is_due_from_its_minute_on() {
		let at = |h, m, s| chrono::NaiveTime::from_hms_opt(h, m, s).unwrap();
		for (now, hour, minute, due) in [
			(at(8, 59, 59), 9, 0, false),
			(at(9, 0, 0), 9, 0, true),
			(at(9, 0, 59), 9, 0, true),
			(at(9, 29, 0), 9, 30, false),
			(at(10, 0, 0), 9, 30, true),
			// a later hour wins even with a smaller minute
			(at(10, 5, 0), 9, 30, true),
			(at(8, 45, 0), 9, 30, false),
			(at(0, 0, 0), 0, 0, true),
			(at(23, 58, 0), 23, 59, false),
			(at(23, 59, 0), 23, 59, true),
		] {
			assert_eq!(
				super::is_due(now, hour, minute),
				due,
				"{now} vs {hour:02}:{minute:02}"
			);
		}
	}

	#[tokio::test]
	async fn a_change_during_a_tick_still_wakes_the_next_wait() {
		// the loop is busy in tick() (not waiting) when the settings
		// change; its next wait must return right away instead of
		// sleeping until the old deadline
		super::notify_settings_changed();
		let woke = tokio::time::timeout(
			Duration::from_millis(200),
			super::REMINDER_SETTINGS_CHANGED.notified(),
		)
		.await;
		assert!(woke.is_ok(), "the wake-up was lost");
	}

	#[test]
	fn raw_next_due_is_never_capped() {
		// 23h59m until tomorrow's reminder is returned uncapped; the
		// loop applies its own 15-minute cap on top
		let late = os_local_wall(2026, 9, 30, 23, 59);
		assert_eq!(until_next_due(None, late, 23, 58).as_secs(), 24 * 3600 - 60);
	}
}
