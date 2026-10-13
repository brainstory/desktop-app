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
