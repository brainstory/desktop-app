//! The read-through AI settings cache: warm reads, one mutation
//! boundary, and the generation counter writers race against.

use std::sync::atomic::Ordering;

use super::{lock, AppState};
use crate::models::AiSettings;

impl AppState {
	/// The AI settings, from the read-through cache when warm. All writes
	/// go through [`Self::mutate_ai_settings`], which publishes under
	/// this same mutex, so the cache can never go stale and a cold read
	/// (load and publish inside the lock) can never overwrite a newer
	/// published value.
	///
	/// The cold read holds the lock across a DB + keychain load. That is
	/// deliberate: loading outside the lock would let a reader observe a
	/// half-committed save (e.g. a new URL with an old secret) in the
	/// window before the saver publishes. The cold path itself is rare -
	/// startup, or after a failed save dropped the cache - and every
	/// normal read hits the warm cache.
	pub fn ai_settings(&self) -> AiSettings {
		let mut cache = lock(&self.ai_settings_cache);
		if let Some(cached) = cache.as_ref() {
			return cached.clone();
		}
		let loaded = AiSettings::load(&self.db);
		*cache = Some(loaded.clone());
		loaded
	}

	/// The one mutation boundary for AI settings: load the latest state
	/// (cache, or database when cold - inside the lock), apply `patch`
	/// to a working copy, persist it, and publish it to the cache, all
	/// under the settings mutex. `patch` returning Err aborts with the
	/// cache and database unchanged.
	///
	/// Persistence is staged (settings row, then each changed secret),
	/// and SQLite cannot transact the OS keychain, so a failed persist
	/// may still have committed part of the update. On any save
	/// failure the cache is therefore re-published from
	/// [`AiSettings::reload`] - the state the stores actually hold -
	/// and the returned Err names the failing stage/secret (never a
	/// secret value). A retry goes through here again and builds on
	/// that reloaded state, so fields committed by the failed attempt
	/// are preserved, never reverted. If the reload itself fails the
	/// cache is dropped (degraded mode: the next read loads cold) and
	/// the Err says the settings state is unavailable - never success
	/// after an incomplete write, never a claimed rollback of
	/// keychain mutations.
	///
	/// Returns the committed snapshot and the new generation (bumped
	/// on every successful publish; a failure-path reconcile publish
	/// does NOT bump it - it reports the stores, it is not a new
	/// commit, and the Err already tells callers to re-read). No
	/// engine work may run while the lock is held - start loaders
	/// after this returns. The critical section may lock the db
	/// connection (settings mutex → db mutex, never the reverse).
	pub fn mutate_ai_settings(
		&self,
		patch: impl FnOnce(&mut AiSettings) -> Result<(), String>,
	) -> Result<(AiSettings, u64), String> {
		let mut cache = lock(&self.ai_settings_cache);
		let latest = match cache.as_ref() {
			Some(cached) => cached.clone(),
			None => {
				let loaded = AiSettings::load(&self.db);
				*cache = Some(loaded.clone());
				loaded
			}
		};
		let mut working = latest.clone();
		// the cache mirrors what is stored, so secrets compare against
		// it (not a fresh keychain read) - untouched secrets are skipped
		// on save
		patch(&mut working)?;
		match working.save(&self.db, &latest) {
			Ok(()) => {
				let generation = self.ai_settings_generation.fetch_add(1, Ordering::SeqCst) + 1;
				*cache = Some(working.clone());
				Ok((working, generation))
			}
			Err(failure) => {
				// The failed save may have committed (the row
				// transaction runs before the secret writes), so the
				// pre-save snapshot is obsolete: publish the
				// authoritative reload so the cache can never serve
				// values the stores no longer hold.
				match AiSettings::reload(&self.db) {
					Ok(authoritative) => {
						*cache = Some(authoritative);
						Err(failure.to_string())
					}
					Err(degraded) => {
						// The store cannot even be read back: drop the
						// cache so the next read loads cold, and say
						// so instead of republishing unverified values.
						*cache = None;
						Err(format!(
							"settings state unavailable after a failed save: {degraded} (failed save: {failure})"
						))
					}
				}
			}
		}
	}
}
