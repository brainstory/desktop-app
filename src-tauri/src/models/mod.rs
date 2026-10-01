//! Models: the pinned catalog, AI settings, runtime state with the
//! engine load/swap machinery, and downloads/hub-cache management.
//! Split from one 2200-line file; everything remains re-exported from
//! here so `crate::models::X` keeps working.

mod ai_settings;
mod catalog;
mod download;
mod state;

pub use ai_settings::{
	default_stt_engine, has_http_scheme, hf_endpoint, resolve_hf_endpoint, AiSettings, LlmMode,
	SpeechEngine,
};
pub use catalog::{find_model, ModelKind, ModelSpec, LLM_MODELS, STT_MODELS};
pub use download::{
	download_model_file, hf_blob_path, hf_cache_model_path, hf_hub_cache_candidates,
	materialize_snapshot, migrate_legacy_models, model_url, primary_hub_cache, remove_cached_model,
	sweep_stale_part_files,
};
pub use state::{AppState, EngineSlotClaim, EngineState, EngineStatus, Runtime};

pub(crate) use download::USER_AGENT;
