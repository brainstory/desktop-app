//! Models: the pinned catalog, AI settings, runtime state with the
//! engine load/swap machinery, and downloads/hub-cache management.
//! Split from one 2200-line file; everything remains re-exported from
//! here so `crate::models::X` keeps working.

mod ai_settings;
mod catalog;
mod download;
mod state;

pub use ai_settings::{
	default_stt_engine, hardened_redirect_policy, hf_endpoint, is_loopback_host,
	resolve_hf_endpoint, same_endpoint_host, secret_host_change, validate_endpoint_url, AiSettings,
	LlmMode, SpeechEngine, SttMode,
};
pub use catalog::{find_model, kv_bytes_per_token, ModelKind, ModelSpec, LLM_MODELS, STT_MODELS};
pub use download::{
	download_model_file, download_model_file_with_pause, hf_blob_path, hf_cache_model_path,
	hf_hub_cache_candidates, materialize_snapshot, migrate_legacy_models, model_url,
	primary_hub_cache, remove_cached_model, remove_staged_part, sweep_stale_part_files,
	verified_snapshot_files, VerifiedSnapshotFiles, DOWNLOAD_CANCELLED, DOWNLOAD_PAUSED,
};
pub use state::{AppState, EngineSlotClaim, EngineState, EngineStatus, PauseEntry, Runtime};

pub(crate) use download::USER_AGENT;
