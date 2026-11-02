//! System notifications: the one send path for the settings test button
//! and the daily reminder.
//!
//! `tauri-plugin-notification`'s `show()` prepares the payload and then
//! fires the actual delivery on a background task whose result it
//! discards - the call resolves `Ok(())` even when the OS refuses, so a
//! denied or suppressed notification was indistinguishable from a sent
//! one (and its permission APIs are desktop stubs that always say
//! "granted"). These helpers send through the same backend the plugin
//! uses (notify-rust) and return the real result: preparation and
//! transport errors surface to the caller.

use tauri::AppHandle;

/// Which bundle id notifications are attributed to: the app's own in a
/// bundled build; Terminal in dev, because `tauri dev` runs an
/// unbundled binary and the legacy notification API only delivers for a
/// bundled, notification-authorized app - the plugin attributes dev
/// notifications to Terminal for exactly that reason. Pure, so the
/// surprising dev attribution is pinned by a test.
fn attribution_identifier(is_dev: bool, configured: &str) -> &str {
	if is_dev {
		"com.apple.Terminal"
	} else {
		configured
	}
}

/// Show a notification and report the real outcome.
///
/// Blocking: delivery on macOS is awaited (bounded by the backend's
/// ~2 s delivery timeout), so call this from the blocking pool, not the
/// main thread or an async worker.
pub fn show(app: &AppHandle, title: &str, body: &str) -> Result<(), String> {
	// resolved on every platform so `app` and the helper stay live
	// outside macOS (only the macOS backend applies an attribution)
	let identifier = attribution_identifier(tauri::is_dev(), &app.config().identifier);
	#[cfg(target_os = "macos")]
	{
		// mirrors the plugin's build(): without this the macOS backend
		// errors with "application not set"
		notify_rust::set_application(identifier).map_err(|e| e.to_string())?;
	}
	// the same intentionally-unused marker apply_presence uses for its
	// macOS-only dock parameter
	#[cfg(not(target_os = "macos"))]
	let _ = identifier;
	let mut notification = notify_rust::Notification::new();
	notification.summary(title).body(body);
	notification.show().map_err(|e| e.to_string()).map(|_| ())
}

#[cfg(test)]
mod tests {
	use super::attribution_identifier;

	#[test]
	fn dev_builds_deliver_as_terminal_bundled_builds_as_the_app() {
		// the dev attribution is load-bearing: an unbundled `tauri dev`
		// binary cannot deliver under its own id, so tests and dev
		// sessions only see notifications while Terminal is allowed to
		// notify (System Settings > Notifications)
		assert_eq!(
			attribution_identifier(true, "ai.brainstory.desktop"),
			"com.apple.Terminal"
		);
		assert_eq!(
			attribution_identifier(false, "ai.brainstory.desktop"),
			"ai.brainstory.desktop"
		);
	}
}
