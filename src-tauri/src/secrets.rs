//! Secret storage: sensitive strings (HuggingFace token, external endpoint
//! API keys) live in the OS keychain via the `keyring` crate. A DB fallback
//! keeps the app working where no keychain service exists (and covers
//! keychain failures): values are stored in the settings table there, which
//! is still local-only, just plaintext.
//!
//! Debug builds never use the keychain. macOS ties a keychain item to the
//! code signature of the app that may read it, and every `tauri dev`
//! rebuild is a new ad-hoc signature, so each start asked for the login
//! password. They keep secrets in the settings table under dev-only keys
//! instead, which a release build's `migrate_from_db` never touches.
//!
//! Secrets are never echoed to the webview - the settings command reports
//! only presence + a masked hint, and save semantics are
//! absent/null = keep, empty string = clear.

use crate::db::Db;

const SERVICE: &str = "ai.brainstory.desktop";

const USE_KEYCHAIN: bool = !cfg!(debug_assertions);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Secret {
	HfToken,
	ExtLlmApiKey,
	ExtSttApiKey,
}

impl Secret {
	/// Keychain account name.
	fn account(self) -> &'static str {
		match self {
			Secret::HfToken => "hf_token",
			Secret::ExtLlmApiKey => "ext_llm_api_key",
			Secret::ExtSttApiKey => "ext_stt_api_key",
		}
	}

	/// Legacy settings-table key (also the DB fallback location).
	pub fn db_key(self) -> &'static str {
		match self {
			Secret::HfToken => crate::keys::setting::secret::HF_TOKEN,
			Secret::ExtLlmApiKey => crate::keys::setting::secret::EXT_LLM_API_KEY,
			Secret::ExtSttApiKey => crate::keys::setting::secret::EXT_STT_API_KEY,
		}
	}

	/// The settings row `load`/`store`/`clear` use: the fallback row in
	/// release builds, a dev-only row (the only location) in debug ones.
	pub(crate) fn store_key(self) -> &'static str {
		if USE_KEYCHAIN {
			return self.db_key();
		}
		match self {
			Secret::HfToken => crate::keys::setting::dev_secret::HF_TOKEN,
			Secret::ExtLlmApiKey => crate::keys::setting::dev_secret::EXT_LLM_API_KEY,
			Secret::ExtSttApiKey => crate::keys::setting::dev_secret::EXT_STT_API_KEY,
		}
	}
}

const ALL: [Secret; 3] = [Secret::HfToken, Secret::ExtLlmApiKey, Secret::ExtSttApiKey];

fn entry(secret: Secret) -> Result<keyring::Entry, String> {
	if !USE_KEYCHAIN {
		return Err("debug builds don't use the keychain".into());
	}
	keyring::Entry::new(SERVICE, secret.account()).map_err(|e| format!("keychain unavailable: {e}"))
}

/// Read a secret: keychain first, then the DB fallback row.
pub fn load(secret: Secret, db: &Db) -> Option<String> {
	let Ok(entry) = entry(secret) else {
		return db.get_setting(secret.store_key());
	};
	match entry.get_password() {
		Ok(value) => Some(value),
		Err(keyring::Error::NoEntry) => db.get_setting(secret.store_key()),
		Err(e) => {
			log::warn!("keychain read for {} failed: {e}", secret.account());
			db.get_setting(secret.store_key())
		}
	}
}

/// Store a secret in the keychain and clear any legacy DB row. On
/// keychain failure, falls back to the DB row so the value is never lost
/// silently (true for a machine without a keychain service).
pub fn store(secret: Secret, value: &str, db: &Db) -> Result<(), String> {
	let fallback = |db: &Db| -> Result<(), String> {
		db.set_setting(secret.store_key(), value)?;
		Ok(())
	};
	let Ok(entry) = entry(secret) else {
		return fallback(db);
	};
	match entry.set_password(value) {
		Ok(()) => {
			// migration complete / no duplicate plaintext copy. A stale
			// row left behind is harmless (the keychain value wins on
			// load), so a failure here is logged, not surfaced.
			if let Err(e) = db.delete_setting(secret.store_key()) {
				log::warn!("{e}");
			}
			Ok(())
		}
		Err(e) => {
			log::warn!(
				"keychain write for {} failed ({e}); storing in local DB instead",
				secret.account()
			);
			fallback(db)
		}
	}
}

/// Remove a secret from both the keychain and the DB fallback. Err means
/// a copy may survive (the next load would bring the "cleared" secret
/// back), so callers must not report success.
pub fn clear(secret: Secret, db: &Db) -> Result<(), String> {
	db.delete_setting(secret.store_key())?;
	// No keychain service means nothing was ever stored there.
	if let Ok(entry) = entry(secret) {
		match entry.delete_credential() {
			Ok(()) | Err(keyring::Error::NoEntry) => {}
			Err(e) => {
				return Err(format!(
					"could not remove {} from the keychain: {e}",
					secret.account()
				))
			}
		}
	}
	Ok(())
}

/// One-time move of any plaintext secret rows into the keychain. Runs at
/// startup before the settings are read; rows only stay behind when the
/// keychain is unusable (the fallback path in `store`).
pub fn migrate_from_db(db: &Db) {
	if !USE_KEYCHAIN {
		return;
	}
	for secret in ALL {
		let Some(value) = db.get_setting(secret.db_key()).filter(|v| !v.is_empty()) else {
			continue;
		};
		let Ok(entry) = entry(secret) else {
			log::warn!(
				"no keychain service; {} stays in the local DB",
				secret.account()
			);
			continue;
		};
		match entry.set_password(&value) {
			Ok(()) => {
				if let Err(e) = db.delete_setting(secret.db_key()) {
					log::warn!("{e}");
				}
				log::info!("moved {} into the keychain", secret.account());
			}
			Err(e) => {
				log::warn!(
					"could not move {} into the keychain ({e}); keeping it in the local DB",
					secret.account()
				);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{Secret, ALL};
	#[cfg(debug_assertions)]
	use {
		super::{clear, load, migrate_from_db, store},
		crate::db::Db,
	};

	#[cfg(debug_assertions)]
	#[test]
	fn debug_builds_keep_secrets_out_of_the_keychain_and_the_release_rows() {
		// unit tests are debug builds: nothing here may reach the keychain
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join("secrets.db")).expect("open");
		// a release build's plaintext fallback row stays its own
		db.set_setting("hf_token", "release-token").unwrap();

		assert_eq!(load(Secret::HfToken, &db), None);
		store(Secret::HfToken, "dev-token", &db).unwrap();
		assert_eq!(db.get_setting("dev_hf_token").as_deref(), Some("dev-token"));
		assert_eq!(load(Secret::HfToken, &db).as_deref(), Some("dev-token"));

		migrate_from_db(&db);
		assert_eq!(db.get_setting("hf_token").as_deref(), Some("release-token"));
		assert_eq!(db.get_setting("dev_hf_token").as_deref(), Some("dev-token"));

		clear(Secret::HfToken, &db).unwrap();
		assert_eq!(load(Secret::HfToken, &db), None);
		assert_eq!(db.get_setting("hf_token").as_deref(), Some("release-token"));
	}

	#[test]
	fn db_keys_are_distinct_and_stable() {
		let mut keys: Vec<&str> = ALL.iter().map(|s| s.db_key()).collect();
		keys.sort_unstable();
		let before = keys.clone();
		keys.dedup();
		assert_eq!(keys.len(), before.len(), "db keys must be unique");
		assert_eq!(Secret::HfToken.db_key(), "hf_token");
	}
}
