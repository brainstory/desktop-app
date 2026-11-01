use tauri::Manager;

pub mod ai;
pub mod data;
pub mod models_cmd;
pub mod reactions;
pub mod settings;
pub mod share;

/// Dispatch a blocking command body: rusqlite and keychain calls are
/// synchronous, so they run on the blocking pool instead of a tokio
/// worker (where a slow disk or keychain read would stall other async
/// work), and a panicked task becomes an error instead of a hung invoke.
pub(crate) async fn run_blocking<T: Send + 'static>(
	label: &'static str,
	work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
	tauri::async_runtime::spawn_blocking(work)
		.await
		.map_err(|e| format!("{label} task failed: {e}"))?
}

/// [`Self::run_blocking`], but the body resolves the AppState itself and
/// every database-writing command goes through it: the body runs only
/// after the shutdown guard passes, so new writes refuse to start once
/// the app began exiting (the exit path waits out in-flight ones instead
/// of cutting them off mid-transaction).
pub(crate) async fn run_blocking_write<T: Send + 'static>(
	app: tauri::AppHandle,
	label: &'static str,
	work: impl FnOnce(&crate::AppState) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
	tauri::async_runtime::spawn_blocking(move || {
		let state = app.state::<crate::AppState>();
		guarded_write(&state, work)
	})
	.await
	.map_err(|e| format!("{label} task failed: {e}"))?
}

/// The guard seam inside every `run_blocking_write` body: refuse once
/// shutdown began, else run the body against the resolved state. Split
/// out so the refusal itself is unit-testable without a Tauri app.
fn guarded_write<T>(
	state: &crate::AppState,
	work: impl FnOnce(&crate::AppState) -> Result<T, String>,
) -> Result<T, String> {
	state.refuse_if_shutting_down()?;
	work(state)
}

#[cfg(test)]
mod shutdown_guard_tests {
	use super::guarded_write;
	use crate::AppState;

	#[test]
	fn a_write_refuses_to_start_once_shutdown_began() {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::create_dir_all(dir.path().join("models")).expect("models dir");
		let db = crate::db::Db::open(&dir.path().join("t.db")).expect("db");
		let state = AppState::new(db, dir.path().to_path_buf());

		// before shutdown a guarded write body runs and writes
		let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let ran_for_body = std::sync::Arc::clone(&ran);
		guarded_write(&state, |state| {
			state
				.db
				.set_setting(crate::keys::setting::USER_NAME, "Ada")?;
			ran_for_body.store(true, std::sync::atomic::Ordering::SeqCst);
			Ok(())
		})
		.expect("write runs before shutdown");
		assert!(ran.load(std::sync::atomic::Ordering::SeqCst));

		// once the flag is set, the body (e.g. a submit_log or settings
		// save) is refused before it can write anything
		state
			.shutting_down
			.store(true, std::sync::atomic::Ordering::SeqCst);
		let ran_for_body = std::sync::Arc::clone(&ran);
		ran.store(false, std::sync::atomic::Ordering::SeqCst);
		let err = guarded_write(&state, |state| {
			state
				.db
				.set_setting(crate::keys::setting::USER_NAME, "late")?;
			ran_for_body.store(true, std::sync::atomic::Ordering::SeqCst);
			Ok(())
		})
		.expect_err("a write must refuse during shutdown");
		assert_eq!(err, "app is shutting down");
		assert!(
			!ran.load(std::sync::atomic::Ordering::SeqCst),
			"the body never ran"
		);
		assert_eq!(
			state
				.db
				.get_setting(crate::keys::setting::USER_NAME)
				.as_deref(),
			Some("Ada"),
			"nothing from the refused write reached the database"
		);
	}
}
