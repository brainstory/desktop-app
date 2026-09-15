//! Secret storage: sensitive strings (HuggingFace token, external endpoint
//! API keys) live in the OS keychain via the `keyring` crate. A DB fallback
//! keeps the app working where no keychain service exists (and covers
//! keychain failures): values are stored in the settings table there, which
//! is still local-only, just plaintext.
//!
//! Secrets are never echoed to the webview - the settings command reports
//! only presence + a masked hint, and save semantics are
//! absent/null = keep, empty string = clear.

use crate::db::Db;

const SERVICE: &str = "ai.brainstory.desktop";

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
		self.account()
	}
}

const ALL: [Secret; 3] = [
	Secret::HfToken,
	Secret::ExtLlmApiKey,
	Secret::ExtSttApiKey,
];

fn entry(secret: Secret) -> Result<keyring::Entry, String> {
	keyring::Entry::new(SERVICE, secret.account()).map_err(|e| format!("keychain unavailable: {e}"))
}

/// Read a secret: keychain first, then the DB fallback row.
pub fn load(secret: Secret, db: &Db) -> Option<String> {
	let Ok(entry) = entry(secret) else {
		return db.get_setting(secret.db_key());
	};
	match entry.get_password() {
		Ok(value) => Some(value),
		Err(keyring::Error::NoEntry) => db.get_setting(secret.db_key()),
		Err(e) => {
			log::warn!("keychain read for {} failed: {e}", secret.account());
			db.get_setting(secret.db_key())
		}
	}
}

/// Store a secret in the keychain and clear any legacy DB row. On
/// keychain failure, falls back to the DB row so the value is never lost
/// silently (true for a machine without a keychain service).
pub fn store(secret: Secret, value: &str, db: &Db) -> Result<(), String> {
	let fallback = |db: &Db| -> Result<(), String> {
		db.set_setting(secret.db_key(), value)?;
		Ok(())
	};
	let Ok(entry) = entry(secret) else {
		return fallback(db);
	};
	match entry.set_password(value) {
		Ok(()) => {
			// migration complete / no duplicate plaintext copy
			db.delete_setting(secret.db_key());
			Ok(())
		}
		Err(e) => {
			log::warn!("keychain write for {} failed ({e}); storing in local DB instead", secret.account());
			fallback(db)
		}
	}
}

/// Remove a secret from both the keychain and the DB fallback.
pub fn clear(secret: Secret, db: &Db) {
	db.delete_setting(secret.db_key());
	if let Ok(entry) = entry(secret) {
		match entry.delete_credential() {
			Ok(()) => {}
			Err(keyring::Error::NoEntry) => {}
			Err(e) => log::warn!("keychain delete for {} failed: {e}", secret.account()),
		}
	}
}

/// One-time move of any plaintext secret rows into the keychain. Runs at
/// startup before the settings are read; rows only stay behind when the
/// keychain is unusable (the fallback path in `store`).
pub fn migrate_from_db(db: &Db) {
	for secret in ALL {
		let Some(value) = db.get_setting(secret.db_key()).filter(|v| !v.is_empty()) else {
			continue;
		};
		let Ok(entry) = entry(secret) else {
			log::warn!("no keychain service; {} stays in the local DB", secret.account());
			continue;
		};
		match entry.set_password(&value) {
			Ok(()) => {
				db.delete_setting(secret.db_key());
				log::info!("moved {} into the keychain", secret.account());
			}
			Err(e) => {
				log::warn!("could not move {} into the keychain ({e}); keeping it in the local DB", secret.account());
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{Secret, ALL};

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
