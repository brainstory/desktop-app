//! Model downloads and the HuggingFace hub cache: verified streaming
//! download with resume (see [`stream`]), and cache discovery, migration
//! and blob/snapshot management (see [`hub_cache`]).

/// User-Agent for all outbound HTTP (downloads, external endpoints).
pub(crate) const USER_AGENT: &str = concat!("brainstory-desktop/", env!("CARGO_PKG_VERSION"));

mod hub_cache;
mod stream;

pub use hub_cache::{
	hf_blob_path, hf_cache_model_path, hf_hub_cache_candidates, materialize_snapshot,
	migrate_legacy_models, model_url, primary_hub_cache, remove_cached_model,
	verified_snapshot_files, VerifiedSnapshotFiles,
};
pub use stream::{
	download_model_file, download_model_file_with_pause, part_path, remove_staged_part,
	sweep_stale_part_files, DOWNLOAD_CANCELLED, DOWNLOAD_PAUSED,
};
