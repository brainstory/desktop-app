//! Model downloads and the HuggingFace hub cache: verified streaming
//! download with resume (see [`stream`]), and cache discovery, migration
//! and blob/snapshot management (see [`hub_cache`]).

/// User-Agent for all outbound HTTP (downloads, external endpoints).
pub(crate) const USER_AGENT: &str = concat!("brainstory-desktop/", env!("CARGO_PKG_VERSION"));

/// One model-download progress event, `kind`-tagged on the wire. Sent
/// per-download over the command channel (without `model_id`/`label`:
/// the channel belongs to one download) and app-wide over the
/// "model-download" event, where those fields name the model for
/// surfaces outside the settings page (the sidebar pill, the chat
/// setup notice).
#[derive(Debug, Clone, PartialEq, serde::Serialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../src/tauri/generated/")]
pub enum DownloadEvent {
	/// bytes landed so far, as a percentage of the model's total size
	#[serde(rename_all = "camelCase")]
	Progress {
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		model_id: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		label: Option<String>,
		pct: f64,
		/// smoothed bytes/sec and the ETA derived from it; absent on the
		/// first reports and while the rate cannot be estimated
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional, type = "number")]
		bytes_per_sec: Option<f64>,
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional, type = "number")]
		eta_secs: Option<u64>,
	},
	/// the transfer finished (the engine may still be loading)
	#[serde(rename_all = "camelCase")]
	Done {
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		model_id: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		label: Option<String>,
	},
	/// the model finished downloading and was set active
	#[serde(rename_all = "camelCase")]
	Activated { model_id: String, label: String },
	/// a user pause stopped the transfer; `pct` is where
	#[serde(rename_all = "camelCase")]
	Paused {
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		model_id: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		label: Option<String>,
		pct: f64,
	},
	/// a user cancel discarded the staged prefix
	#[serde(rename_all = "camelCase")]
	Cancelled {
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		model_id: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		label: Option<String>,
	},
	/// the transfer failed
	#[serde(rename_all = "camelCase")]
	Error {
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		model_id: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		label: Option<String>,
		message: String,
	},
	/// the download succeeded but activating/loading the model failed
	#[serde(rename = "load-error", rename_all = "camelCase")]
	LoadError {
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		model_id: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		#[ts(optional)]
		label: Option<String>,
		message: String,
	},
}

mod hub_cache;
mod stream;

pub use hub_cache::{
	hf_blob_path, hf_cache_model_path, hf_hub_cache_candidates, materialize_snapshot,
	materialize_snapshot_in, migrate_legacy_models, model_url, primary_hub_cache,
	remove_cached_model, verified_snapshot_files, VerifiedSnapshotFiles,
};
pub use stream::{
	download_model_file, download_model_file_with_pause, part_path, remove_staged_part,
	sweep_stale_part_files, DOWNLOAD_CANCELLED, DOWNLOAD_PAUSED,
};
