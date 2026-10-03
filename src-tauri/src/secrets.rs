//! Secret storage: sensitive strings (HuggingFace token, external endpoint
//! API keys) live in the OS keychain via the `keyring` crate. The keychain
//! is authoritative whenever it holds a value: a DB fallback keeps the app
//! working where no keychain service exists, or where the service exists
//! but verifiably holds nothing and cannot be written - values are stored
//! in the settings table there, which is still local-only, just plaintext.
//! A keychain that holds (or may hold) an older value never gains a
//! fallback copy a later load would silently ignore.
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

use std::cell::RefCell;
use std::rc::Rc;

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

/// The keychain as a per-account function table, so tests can run the
/// exact store/load/clear/migrate algorithm against a scripted fake
/// instead of an OS keychain (debug builds must never touch the real
/// one). Production always uses [`real_keychain`].
type AvailableFn = Box<dyn Fn(&str) -> bool>;
type GetFn = Box<dyn Fn(&str) -> Result<Option<String>, String>>;
type SetFn = Box<dyn Fn(&str, &str) -> Result<(), String>>;
type DeleteFn = Box<dyn Fn(&str) -> Result<(), String>>;

struct KeychainBackend {
	/// The one shared no-service determination: whether the keychain
	/// entry for this account can be constructed at all. When it says
	/// no, the DB row is the only store and nothing else in this table
	/// is called - for store, load and clear alike.
	available: AvailableFn,
	/// `Ok(Some(value))` = an entry exists; `Ok(None)` = the keychain
	/// definitely holds nothing (`NoEntry`); `Err` = the read failed.
	get: GetFn,
	set: SetFn,
	/// `Ok` covers both "deleted" and "was already absent" (`NoEntry`).
	delete: DeleteFn,
}

thread_local! {
	/// The injected fake backend for this thread. Only tests write
	/// here; it is always empty in production, so release builds talk
	/// to the real keychain and debug builds report no service.
	static INJECTED_BACKEND: RefCell<Option<Rc<KeychainBackend>>> = const { RefCell::new(None) };
}

/// The real OS keychain via `keyring`. Every operation constructs its
/// own entry; if construction fails inside an operation after the
/// availability probe passed, that operation simply reports a failure
/// (the keychain state could not be verified either way).
fn real_keychain() -> KeychainBackend {
	fn entry(account: &str) -> Result<keyring::Entry, String> {
		keyring::Entry::new(SERVICE, account).map_err(|e| format!("keychain unavailable: {e}"))
	}
	KeychainBackend {
		available: Box::new(|account| keyring::Entry::new(SERVICE, account).is_ok()),
		get: Box::new(|account| match entry(account)?.get_password() {
			Ok(value) => Ok(Some(value)),
			Err(keyring::Error::NoEntry) => Ok(None),
			Err(e) => Err(format!("keychain read failed: {e}")),
		}),
		set: Box::new(|account, value| {
			entry(account)?
				.set_password(value)
				.map_err(|e| format!("keychain write failed: {e}"))
		}),
		delete: Box::new(|account| match entry(account)?.delete_credential() {
			Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
			Err(e) => Err(format!("keychain delete failed: {e}")),
		}),
	}
}

/// The keychain service for this secret: the test-injected fake, or
/// (release builds only) the real keyring. `None` means no keychain
/// service exists for this account, so the DB row is the only store
/// and is authoritative for store, load and clear alike. This one
/// determination is shared by every operation in this module.
fn keychain_service(secret: Secret) -> Option<Rc<KeychainBackend>> {
	let backend = INJECTED_BACKEND
		.with(|slot| slot.borrow().clone())
		.or_else(|| {
			if USE_KEYCHAIN {
				Some(Rc::new(real_keychain()))
			} else {
				None
			}
		})?;
	if !(backend.available)(secret.account()) {
		return None;
	}
	Some(backend)
}

/// Read a secret: keychain first, then the DB fallback row.
pub fn load(secret: Secret, db: &Db) -> Option<String> {
	let Some(backend) = keychain_service(secret) else {
		return db.get_setting(secret.store_key());
	};
	match (backend.get)(secret.account()) {
		Ok(Some(value)) => Some(value),
		Ok(None) => db.get_setting(secret.store_key()),
		Err(e) => {
			log::warn!("keychain read for {} failed: {e}", secret.account());
			db.get_setting(secret.store_key())
		}
	}
}

/// Store a secret in the keychain and clear any legacy DB row. With
/// no keychain service the DB row is the only store, so the value
/// goes there. When the service exists but the write fails, the
/// keychain is read back first: `load` keeps preferring the keychain,
/// so a DB fallback is safe only while the keychain verifiably holds
/// nothing. An entry that survived the failed write (the old value)
/// would silently ignore a "successful" fallback forever, so the
/// store is refused instead - the old value stays everywhere, which
/// is the one state a later `load` reports truthfully.
pub fn store(secret: Secret, value: &str, db: &Db) -> Result<(), String> {
	let account = secret.account();
	let Some(backend) = keychain_service(secret) else {
		return db.set_setting(secret.store_key(), value);
	};
	match (backend.set)(account, value) {
		Ok(()) => {
			// migration complete / no duplicate plaintext copy. A stale
			// row left behind is harmless (the keychain value wins on
			// load), so a failure here is logged, not surfaced.
			if let Err(e) = db.delete_setting(secret.store_key()) {
				log::warn!("{e}");
			}
			Ok(())
		}
		Err(set_err) => {
			log::warn!("keychain write for {account} failed ({set_err})");
			// An unreadable keychain is refused too: absent (safe to
			// fall back) and inaccessible (an old value may survive
			// unseen) cannot be told apart.
			match (backend.get)(account) {
				Ok(None) => db.set_setting(secret.store_key(), value),
				Ok(Some(_)) => Err(format!(
					"could not update {account} in the keychain ({set_err}); an earlier value is still stored there, so nothing was changed"
				)),
				Err(read_err) => Err(format!(
					"could not update {account} in the keychain ({set_err}), and could not confirm the keychain is empty ({read_err}); nothing was changed"
				)),
			}
		}
	}
}

/// Remove a secret from both the keychain and the DB fallback. Err means
/// a copy may survive (the next load would bring the "cleared" secret
/// back), so callers must not report success.
pub fn clear(secret: Secret, db: &Db) -> Result<(), String> {
	db.delete_setting(secret.store_key())?;
	// No keychain service means nothing was ever stored there.
	let Some(backend) = keychain_service(secret) else {
		return Ok(());
	};
	match (backend.delete)(secret.account()) {
		Ok(()) => Ok(()),
		Err(e) => Err(format!(
			"could not remove {} from the keychain: {e}",
			secret.account()
		)),
	}
}

/// One-time move of any plaintext secret rows into the keychain.
/// Runs at startup before the settings are read. An existing keychain
/// entry is NEVER overwritten: a row can survive a failed cleanup
/// after a later keychain store, and rewriting it would revert the
/// newer keychain value on every restart. Instead the keychain is
/// read first - empty entries receive the row's value (today's
/// migration), entries that already exist make the row a stale
/// duplicate that is deleted without touching the keychain, and an
/// unreadable keychain leaves the row in place for the next startup.
pub fn migrate_from_db(db: &Db) {
	for secret in ALL {
		let account = secret.account();
		let Some(value) = db.get_setting(secret.db_key()).filter(|v| !v.is_empty()) else {
			continue;
		};
		let Some(backend) = keychain_service(secret) else {
			log::warn!("no keychain service; {account} stays in the local DB");
			continue;
		};
		match (backend.get)(account) {
			Ok(None) => match (backend.set)(account, &value) {
				Ok(()) => {
					if let Err(e) = db.delete_setting(secret.db_key()) {
						log::warn!("{e}");
					}
					log::info!("moved {account} into the keychain");
				}
				Err(e) => {
					log::warn!(
						"could not move {account} into the keychain ({e}); keeping it in the local DB"
					);
				}
			},
			Ok(Some(_)) => {
				if let Err(e) = db.delete_setting(secret.db_key()) {
					log::warn!("{e}");
				}
				log::info!("dropped a stale local row for {account}; the keychain value wins");
			}
			Err(e) => {
				log::warn!(
					"could not read the keychain for {account} ({e}); keeping it in the local DB"
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
		super::{clear, load, migrate_from_db, store, KeychainBackend, INJECTED_BACKEND},
		crate::db::Db,
		std::cell::RefCell,
		std::collections::{HashMap, HashSet},
		std::rc::Rc,
	};

	#[cfg(debug_assertions)]
	fn temp_db(name: &str) -> (Db, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join(format!("{name}.db"))).expect("open test db");
		(db, dir)
	}

	/// A scriptable fake keychain: which account holds what, plus
	/// per-account failure switches. Each regression forces exactly one
	/// operation to fail (or the whole service to be absent for an
	/// account) and then inspects what was actually written - the real
	/// OS keychain is never touched.
	#[cfg(debug_assertions)]
	#[derive(Default)]
	struct Fake {
		values: RefCell<HashMap<String, String>>,
		unavailable: RefCell<HashSet<String>>,
		failing_get: RefCell<HashSet<String>>,
		failing_set: RefCell<HashSet<String>>,
		failing_delete: RefCell<HashSet<String>>,
	}

	#[cfg(debug_assertions)]
	impl Fake {
		fn seed(&self, account: &str, value: &str) {
			self.values
				.borrow_mut()
				.insert(account.into(), value.into());
		}

		fn fail_set(&self, account: &str) {
			self.failing_set.borrow_mut().insert(account.into());
		}

		fn fail_get(&self, account: &str) {
			self.failing_get.borrow_mut().insert(account.into());
		}

		fn fail_delete(&self, account: &str) {
			self.failing_delete.borrow_mut().insert(account.into());
		}

		fn make_unavailable(&self, account: &str) {
			self.unavailable.borrow_mut().insert(account.into());
		}

		fn value(&self, account: &str) -> Option<String> {
			self.values.borrow().get(account).cloned()
		}

		fn backend(self: &Rc<Self>) -> KeychainBackend {
			let available = Rc::clone(self);
			let get = Rc::clone(self);
			let set = Rc::clone(self);
			let delete = Rc::clone(self);
			KeychainBackend {
				available: Box::new(move |account| {
					!available.unavailable.borrow().contains(account)
				}),
				get: Box::new(move |account| {
					if get.failing_get.borrow().contains(account) {
						return Err("simulated keychain read failure".into());
					}
					Ok(get.values.borrow().get(account).cloned())
				}),
				set: Box::new(move |account, value| {
					if set.failing_set.borrow().contains(account) {
						return Err("simulated keychain write failure".into());
					}
					set.values.borrow_mut().insert(account.into(), value.into());
					Ok(())
				}),
				delete: Box::new(move |account| {
					if delete.failing_delete.borrow().contains(account) {
						return Err("simulated keychain delete failure".into());
					}
					delete.values.borrow_mut().remove(account);
					Ok(())
				}),
			}
		}
	}

	/// Install `backend` as this thread's keychain for `body`. The seam
	/// is cleared on the way out (even through a panic) so the next
	/// test on this pooled thread cannot see this fake.
	#[cfg(debug_assertions)]
	fn with_fake_backend<R>(backend: KeychainBackend, body: impl FnOnce() -> R) -> R {
		INJECTED_BACKEND.with(|slot| *slot.borrow_mut() = Some(Rc::new(backend)));
		struct Reset;
		impl Drop for Reset {
			fn drop(&mut self) {
				INJECTED_BACKEND.with(|slot| *slot.borrow_mut() = None);
			}
		}
		let _reset = Reset;
		body()
	}

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

	#[cfg(debug_assertions)]
	#[test]
	fn load_prefers_the_keychain_when_both_stores_hold_a_value() {
		let (db, _dir) = temp_db("load-precedence");
		let fake = Rc::new(Fake::default());
		fake.seed("hf_token", "keychain-value");
		db.set_setting(Secret::HfToken.store_key(), "db-value")
			.unwrap();
		with_fake_backend(fake.backend(), || {
			assert_eq!(
				load(Secret::HfToken, &db).as_deref(),
				Some("keychain-value"),
				"the keychain is authoritative when both copies exist"
			);
		});
	}

	#[cfg(debug_assertions)]
	#[test]
	fn store_refuses_the_db_fallback_while_the_keychain_still_holds_a_value() {
		// a keychain write fails but the old entry survived it: load
		// keeps preferring the keychain, so a "successful" DB fallback
		// would be silently ignored forever - store must refuse
		let (db, _dir) = temp_db("store-refuse-stale");
		let fake = Rc::new(Fake::default());
		fake.seed("hf_token", "old-value");
		fake.fail_set("hf_token");
		with_fake_backend(fake.backend(), || {
			let err = store(Secret::HfToken, "new-value", &db)
				.expect_err("a fallback every future load would ignore must not report success");
			assert!(err.contains("keychain"), "the error names the store: {err}");
			assert!(
				!err.contains("new-value") && !err.contains("old-value"),
				"no secret value in errors: {err}"
			);
			// nothing changed anywhere: the old value everywhere is the
			// one consistent state
			assert_eq!(fake.value("hf_token").as_deref(), Some("old-value"));
			assert_eq!(
				db.get_setting(Secret::HfToken.store_key()),
				None,
				"no divergent fallback copy was written"
			);
			assert_eq!(load(Secret::HfToken, &db).as_deref(), Some("old-value"));
		});
	}

	#[cfg(debug_assertions)]
	#[test]
	fn store_refuses_the_db_fallback_when_it_cannot_read_the_keychain_back() {
		// write fails and the verification read fails too: absent (safe
		// to fall back) and inaccessible (an old value may survive
		// unseen) can no longer be told apart, so refuse
		let (db, _dir) = temp_db("store-refuse-unverifiable");
		let fake = Rc::new(Fake::default());
		fake.fail_set("hf_token");
		fake.fail_get("hf_token");
		with_fake_backend(fake.backend(), || {
			store(Secret::HfToken, "new-value", &db)
				.expect_err("an unverifiable keychain must not gain a DB shadow copy");
			assert_eq!(db.get_setting(Secret::HfToken.store_key()), None);
			assert_eq!(fake.value("hf_token"), None);
		});
	}

	#[cfg(debug_assertions)]
	#[test]
	fn store_falls_back_to_the_db_row_when_the_keychain_is_verifiably_empty() {
		// a machine whose keychain exists but cannot be written (and
		// verifiably holds nothing): the DB row is a safe fallback
		let (db, _dir) = temp_db("store-fallback-empty");
		let fake = Rc::new(Fake::default());
		fake.fail_set("hf_token");
		with_fake_backend(fake.backend(), || {
			store(Secret::HfToken, "fallback-value", &db).expect("safe fallback");
			assert_eq!(
				db.get_setting(Secret::HfToken.store_key()).as_deref(),
				Some("fallback-value")
			);
			assert_eq!(fake.value("hf_token"), None, "nothing reached the keychain");
			assert_eq!(
				load(Secret::HfToken, &db).as_deref(),
				Some("fallback-value")
			);
		});
	}

	#[cfg(debug_assertions)]
	#[test]
	fn store_reports_a_failed_db_fallback_write() {
		// keychain write fails against a verifiably empty keychain, but
		// the DB row write fails too: no copy exists anywhere, and the
		// caller must hear about it
		let dir = tempfile::tempdir().expect("tempdir");
		let path = dir.path().join("fallback-write-fails.db");
		let db = Db::open(&path).expect("open");
		let row = Secret::HfToken.store_key();
		{
			let conn = rusqlite::Connection::open(&path).unwrap();
			conn.execute_batch(&format!(
				"CREATE TRIGGER block_fallback BEFORE INSERT ON settings
				 WHEN NEW.key = '{row}'
				 BEGIN SELECT RAISE(ABORT, 'database is locked'); END;"
			))
			.unwrap();
		}
		let fake = Rc::new(Fake::default());
		fake.fail_set("hf_token");
		with_fake_backend(fake.backend(), || {
			let err = store(Secret::HfToken, "fallback-value", &db)
				.expect_err("a secret stored nowhere must not report success");
			assert!(
				err.contains("failed to save setting"),
				"the store's own error surfaces: {err}"
			);
			assert_eq!(load(Secret::HfToken, &db), None);
		});
	}

	#[cfg(debug_assertions)]
	#[test]
	fn migration_never_overwrites_a_newer_keychain_value() {
		// A successful keychain store whose row cleanup failed
		// leaves the old release row behind; the next startup's
		// migration must delete that stale row, not write it back over
		// the newer keychain value
		let (db, _dir) = temp_db("migrate-no-revert");
		db.set_setting(Secret::HfToken.db_key(), "older-value")
			.expect("seed stale release row");
		let fake = Rc::new(Fake::default());
		with_fake_backend(fake.backend(), || {
			store(Secret::HfToken, "newer-value", &db).expect("keychain store");
			// the keychain now holds the newer value; in a release build
			// store() would have deleted the release row here - its
			// survival (debug keeps the fallback in the dev-only row,
			// so db_key is untouched) is exactly a failed cleanup
			assert_eq!(load(Secret::HfToken, &db).as_deref(), Some("newer-value"));

			migrate_from_db(&db);
			assert_eq!(
				fake.value("hf_token").as_deref(),
				Some("newer-value"),
				"the stale row must not revert the keychain value"
			);
			assert_eq!(
				db.get_setting(Secret::HfToken.db_key()),
				None,
				"the stale duplicate row is deleted"
			);
			assert_eq!(load(Secret::HfToken, &db).as_deref(), Some("newer-value"));

			// idempotent: migrating again changes nothing
			migrate_from_db(&db);
			assert_eq!(fake.value("hf_token").as_deref(), Some("newer-value"));
			assert_eq!(db.get_setting(Secret::HfToken.db_key()), None);
			assert_eq!(load(Secret::HfToken, &db).as_deref(), Some("newer-value"));
		});
	}

	#[cfg(debug_assertions)]
	#[test]
	fn migration_keeps_the_row_when_it_cannot_read_the_keychain() {
		// unreadable keychain: overwriting risks reverting a newer
		// value, so the row waits for a startup that can read it
		let (db, _dir) = temp_db("migrate-unreadable");
		db.set_setting(Secret::HfToken.db_key(), "legacy-value")
			.expect("seed row");
		let fake = Rc::new(Fake::default());
		fake.fail_get("hf_token");
		with_fake_backend(fake.backend(), || {
			migrate_from_db(&db);
			assert_eq!(
				db.get_setting(Secret::HfToken.db_key()).as_deref(),
				Some("legacy-value"),
				"the row must stay put"
			);
			assert_eq!(fake.value("hf_token"), None);
		});
	}

	#[cfg(debug_assertions)]
	#[test]
	fn migration_moves_a_plaintext_row_into_an_empty_keychain_once() {
		// the original migration: row present, keychain empty -> move
		// and delete the row; a second migration is a no-op
		let (db, _dir) = temp_db("migrate-legit");
		db.set_setting(Secret::HfToken.db_key(), "legacy-value")
			.expect("seed row");
		let fake = Rc::new(Fake::default());
		with_fake_backend(fake.backend(), || {
			migrate_from_db(&db);
			assert_eq!(fake.value("hf_token").as_deref(), Some("legacy-value"));
			assert_eq!(db.get_setting(Secret::HfToken.db_key()), None);
			assert_eq!(load(Secret::HfToken, &db).as_deref(), Some("legacy-value"));

			migrate_from_db(&db);
			assert_eq!(fake.value("hf_token").as_deref(), Some("legacy-value"));
			assert_eq!(db.get_setting(Secret::HfToken.db_key()), None);
			assert_eq!(load(Secret::HfToken, &db).as_deref(), Some("legacy-value"));
		});
	}

	#[cfg(debug_assertions)]
	#[test]
	fn clear_treats_a_keychain_that_cannot_be_constructed_as_empty() {
		// entry construction fails: there is no keychain service, so
		// nothing was ever stored there and clearing the DB row alone
		// is truthful. fail_delete proves the delete operation is
		// never reached.
		let (db, _dir) = temp_db("clear-no-service");
		let fake = Rc::new(Fake::default());
		fake.seed("hf_token", "leftover-from-an-earlier-script");
		fake.make_unavailable("hf_token");
		fake.fail_delete("hf_token");
		db.set_setting(Secret::HfToken.store_key(), "old-value")
			.expect("seed row");
		with_fake_backend(fake.backend(), || {
			clear(Secret::HfToken, &db).expect("the DB row was the only copy");
			assert_eq!(load(Secret::HfToken, &db), None);
		});
	}

	#[cfg(debug_assertions)]
	#[test]
	fn clear_fails_only_when_a_keychain_delete_really_fails() {
		let (db, _dir) = temp_db("clear-delete-fails");
		let fake = Rc::new(Fake::default());
		fake.seed("hf_token", "old-value");
		fake.fail_delete("hf_token");
		with_fake_backend(fake.backend(), || {
			let err = clear(Secret::HfToken, &db)
				.expect_err("a keychain copy that survives must not report success");
			assert!(
				err.contains("could not remove"),
				"the surviving store is named: {err}"
			);
			assert_eq!(
				load(Secret::HfToken, &db).as_deref(),
				Some("old-value"),
				"the surviving copy is still authoritative"
			);
		});

		// an absent keychain entry clears successfully
		let absent = Rc::new(Fake::default());
		with_fake_backend(absent.backend(), || {
			clear(Secret::HfToken, &db).expect("nothing to delete is success");
			assert_eq!(load(Secret::HfToken, &db), None);
		});
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
