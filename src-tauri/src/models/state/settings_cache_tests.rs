use super::super::catalog::LLM_MODELS;
use super::*;

fn temp_state(name: &str) -> (AppState, tempfile::TempDir) {
	let dir = tempfile::tempdir().expect("tempdir");
	let db = Db::open(&dir.path().join(format!("{name}.db"))).expect("db");
	(AppState::new(db, dir.path().to_path_buf()), dir)
}

/// Reopen the same database behind a fresh AppState, so assertions
/// check what was persisted, not just the cache.
fn reopened(dir: &std::path::Path, name: &str) -> AppState {
	let db = Db::open(&dir.join(format!("{name}.db"))).expect("reopen db");
	AppState::new(db, dir.to_path_buf())
}

#[test]
fn ai_settings_cache_round_trips_through_mutate() {
	let (state, dir) = temp_state("roundtrip");

	// cold read loads and warms the cache
	assert_eq!(state.ai_settings().llm_model, LLM_MODELS[0].id);
	// a write through the mutation boundary refreshes the cache
	state
		.mutate_ai_settings(|s| {
			s.stt_language = "fr-FR".into();
			Ok(())
		})
		.expect("mutate");
	assert_eq!(state.ai_settings().stt_language, "fr-FR");
	// and persisted: a fresh AppState sees the same value
	assert_eq!(
		reopened(dir.path(), "roundtrip").ai_settings().stt_language,
		"fr-FR"
	);
}
#[test]
fn ai_settings_cache_is_a_cache_not_a_source() {
	// the cache only ever mirrors committed database writes (via
	// mutate_ai_settings), proving the cache never outruns the
	// database
	let (state, _dir) = temp_state("mirror");
	let _ = state.ai_settings(); // warm
	state
		.mutate_ai_settings(|s| {
			s.llm_model = "gemma-4-E4B".into();
			Ok(())
		})
		.expect("mutate");
	assert_eq!(state.ai_settings().llm_model, "gemma-4-E4B");
}

#[test]
fn disjoint_concurrent_patches_preserve_each_others_fields() {
	let (state, dir) = temp_state("disjoint");
	let state = std::sync::Arc::new(state);

	// Both writers are poised before either mutates (the barrier),
	// and the extLlmModel writer commits first (the channel), so the
	// sttLanguage writer lands last. Committing last must not roll
	// the other field back: each patch applies to the LATEST state
	// under the mutex, never to a stale caller-side snapshot.
	let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
	let (tx, rx) = std::sync::mpsc::channel::<()>();

	let a_state = state.clone();
	let a_barrier = barrier.clone();
	let a = std::thread::spawn(move || {
		a_barrier.wait();
		a_state
			.mutate_ai_settings(|s| {
				s.apply_updates(&serde_json::json!({ "extLlmModel": "new-model" }))
			})
			.expect("extLlmModel patch commits");
		tx.send(()).expect("first writer alive");
	});
	let b_state = state.clone();
	let b = std::thread::spawn(move || {
		barrier.wait();
		rx.recv().expect("first writer committed");
		b_state
			.mutate_ai_settings(|s| s.apply_updates(&serde_json::json!({ "sttLanguage": "fr-FR" })))
			.expect("sttLanguage patch commits");
	});
	a.join().unwrap();
	b.join().unwrap();

	assert_eq!(state.ai_settings().ext_llm_model, "new-model");
	assert_eq!(state.ai_settings().stt_language, "fr-FR");
	assert_eq!(
		reopened(dir.path(), "disjoint").ai_settings().ext_llm_model,
		"new-model"
	);
	assert_eq!(
		reopened(dir.path(), "disjoint").ai_settings().stt_language,
		"fr-FR"
	);
}

#[test]
fn same_field_concurrent_patches_serialize_last_committer_wins() {
	let (state, dir) = temp_state("samefield");
	let state = std::sync::Arc::new(state);

	let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
	let a_state = state.clone();
	let a_barrier = barrier.clone();
	let a = std::thread::spawn(move || {
		a_barrier.wait();
		a_state
			.mutate_ai_settings(|s| {
				s.stt_language = "de-DE".into();
				Ok(())
			})
			.expect("first same-field patch")
	});
	let b_state = state.clone();
	let b = std::thread::spawn(move || {
		barrier.wait();
		b_state
			.mutate_ai_settings(|s| {
				s.stt_language = "fr-FR".into();
				Ok(())
			})
			.expect("second same-field patch")
	});
	let a_result = a.join().unwrap();
	let b_result = b.join().unwrap();

	// the higher generation committed last; its value is what both
	// the cache and the database must show
	let (last_value, last_gen) = if a_result.1 > b_result.1 {
		("de-DE", a_result.1)
	} else {
		("fr-FR", b_result.1)
	};
	assert!(last_gen >= 2, "each commit bumped the generation");
	assert_eq!(state.ai_settings().stt_language, last_value);
	assert_eq!(
		reopened(dir.path(), "samefield").ai_settings().stt_language,
		last_value
	);
}

#[test]
fn an_invalid_patch_changes_neither_cache_nor_database() {
	let (state, dir) = temp_state("invalid");
	let before = state.ai_settings();
	let gen_before = state.ai_settings_generation.load(Ordering::SeqCst);

	let err = state
		.mutate_ai_settings(|s| {
			s.apply_updates(&serde_json::json!({
				"llmModel": "not-a-model",
				"sttLanguage": "de-DE",
			}))
		})
		.expect_err("the invalid field rejects the whole patch");
	assert!(err.contains("unknown llmModel"), "unexpected: {err}");

	let after = state.ai_settings();
	assert_eq!(after.llm_model, before.llm_model);
	assert_eq!(
		after.stt_language, before.stt_language,
		"de-DE must not leak"
	);
	assert_eq!(
		state.ai_settings_generation.load(Ordering::SeqCst),
		gen_before,
		"a rejected patch is no publish"
	);
	let persisted = reopened(dir.path(), "invalid").ai_settings();
	assert_eq!(persisted.llm_model, before.llm_model);
	assert_eq!(persisted.stt_language, before.stt_language);
}

#[test]
fn a_cold_read_racing_a_save_never_loses_the_save() {
	// Fresh state per round so every read is the cold one; whichever
	// side gets the mutex first, the committed value must survive in
	// cache and database.
	for round in 0..32 {
		let (state, dir) = temp_state(&format!("coldrace{round}"));
		let state = std::sync::Arc::new(state);
		let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

		let a_state = state.clone();
		let a_barrier = barrier.clone();
		let a = std::thread::spawn(move || {
			a_barrier.wait();
			a_state.ai_settings();
		});
		let b_state = state.clone();
		let b = std::thread::spawn(move || {
			barrier.wait();
			b_state
				.mutate_ai_settings(|s| {
					s.apply_updates(&serde_json::json!({ "sttLanguage": "fr-FR" }))
				})
				.expect("save commits");
		});
		a.join().unwrap();
		b.join().unwrap();

		assert_eq!(
			state.ai_settings().stt_language,
			"fr-FR",
			"round {round}: the cold read overwrote the published value"
		);
		assert_eq!(
			reopened(dir.path(), &format!("coldrace{round}"))
				.ai_settings()
				.stt_language,
			"fr-FR",
			"round {round}"
		);
	}
}

#[test]
fn mutate_preserves_secret_keep_and_clear_semantics() {
	// debug builds keep secrets in dev-only DB rows, so this never
	// touches the developer's real keychain
	let (state, dir) = temp_state("secrets");
	state
		.mutate_ai_settings(|s| {
			s.apply_updates(&serde_json::json!({ "hfToken": "fake-test-token" }))
		})
		.expect("store token");
	assert_eq!(state.ai_settings().hf_token, "fake-test-token");

	// an absent field keeps the stored secret
	state
		.mutate_ai_settings(|s| s.apply_updates(&serde_json::json!({ "sttLanguage": "fr-FR" })))
		.expect("patch without the token field");
	assert_eq!(state.ai_settings().hf_token, "fake-test-token");
	// and so does an explicit null
	state
		.mutate_ai_settings(|s| {
			s.apply_updates(&serde_json::json!({ "sttLanguage": "de-DE", "hfToken": null }))
		})
		.expect("null token");
	assert_eq!(state.ai_settings().hf_token, "fake-test-token");

	// an explicit empty string clears it
	state
		.mutate_ai_settings(|s| s.apply_updates(&serde_json::json!({ "hfToken": "" })))
		.expect("clear token");
	assert_eq!(state.ai_settings().hf_token, "");
	assert_eq!(reopened(dir.path(), "secrets").ai_settings().hf_token, "");
}

// ---- partial persistence failure ----
//
// Ordinary rows commit in one transaction BEFORE the sequential
// secret writes, so a failing secret leaves the database holding
// the new ordinary values while a stale cache still serves the old
// ones. These regressions inject failures via SQLite triggers on
// dev-only rows (debug builds keep secrets there), so they never
// touch the developer's real keychain.

/// Abort writes of one settings key behind Db's back (BEFORE INSERT
/// covers the upsert's insert path for a row that does not exist
/// yet; BEFORE DELETE covers a secret clear).
fn fail_key_writes(path: &std::path::Path, key: &str, message: &str) {
	let conn = rusqlite::Connection::open(path).unwrap();
	conn.execute_batch(&format!(
		"CREATE TRIGGER fail_{key} BEFORE INSERT ON settings
			 WHEN NEW.key = '{key}'
			 BEGIN SELECT RAISE(ABORT, '{message}'); END;"
	))
	.unwrap();
}

fn fail_key_deletes(path: &std::path::Path, key: &str, message: &str) {
	let conn = rusqlite::Connection::open(path).unwrap();
	conn.execute_batch(&format!(
		"CREATE TRIGGER keep_{key} BEFORE DELETE ON settings
			 WHEN OLD.key = '{key}'
			 BEGIN SELECT RAISE(ABORT, '{message}'); END;"
	))
	.unwrap();
}

fn drop_fail_key_trigger(path: &std::path::Path, key: &str) {
	let conn = rusqlite::Connection::open(path).unwrap();
	conn.execute_batch(&format!("DROP TRIGGER fail_{key};"))
		.unwrap();
}

#[test]
fn failed_secret_save_leaves_the_cache_on_the_committed_state() {
	// The settings row commits, then the token write fails.
	// The cache must not keep serving the pre-save snapshot the
	// database no longer holds.
	let (state, dir) = temp_state("f06-truth");
	let db_path = dir.path().join("f06-truth.db");
	let _ = state.ai_settings(); // warm the cache
	fail_key_writes(
		&db_path,
		crate::keys::setting::dev_secret::HF_TOKEN,
		"injected secret write failure",
	);

	state
		.mutate_ai_settings(|s| {
			s.apply_updates(&serde_json::json!({
				"sttLanguage": "fr-FR",
				"extLlmModel": "changed-before-error",
				"hfToken": "fake-test-token",
			}))
		})
		.expect_err("the token write fails");

	// the cache reflects what the stores actually hold now
	assert_eq!(state.ai_settings().stt_language, "fr-FR");
	assert_eq!(state.ai_settings().ext_llm_model, "changed-before-error");
	assert_eq!(state.ai_settings().hf_token, "");
	// cold-restart equivalence: a fresh AppState loads the same state
	let cold = reopened(dir.path(), "f06-truth");
	assert_eq!(cold.ai_settings().stt_language, "fr-FR");
	assert_eq!(cold.ai_settings().ext_llm_model, "changed-before-error");
	assert_eq!(cold.ai_settings().hf_token, "");
}

#[test]
fn secret_failures_name_the_failed_secret_and_keep_later_secrets_unwritten() {
	use crate::keys::setting::dev_secret;
	// (dev row to break, secret's error name) in the order save()
	// writes them; every round's patch touches all three secrets
	let cases = [
		(dev_secret::HF_TOKEN, "HuggingFace token"),
		(dev_secret::EXT_LLM_API_KEY, "external LLM API key"),
		(dev_secret::EXT_STT_API_KEY, "external STT API key"),
	];
	for (round, (row, name)) in cases.iter().enumerate() {
		let (state, dir) = temp_state(&format!("f06-name{round}"));
		let db_path = dir.path().join(format!("f06-name{round}.db"));
		let _ = state.ai_settings();
		fail_key_writes(&db_path, row, "injected secret write failure");

		let err = state
			.mutate_ai_settings(|s| {
				s.apply_updates(&serde_json::json!({
					"sttLanguage": "fr-FR",
					"hfToken": "fake-hf-token",
					"extLlmApiKey": "fake-llm-key",
					"extSttApiKey": "fake-stt-key",
				}))
			})
			.expect_err("one secret write fails");
		assert!(
			err.contains(&format!("storing the {name} failed")),
			"round {round}: the error must name the failed secret: {err}"
		);
		assert!(
			err.contains("injected secret write failure"),
			"round {round}: the store's own error must surface: {err}"
		);
		assert!(
			!err.contains("fake-hf-token")
				&& !err.contains("fake-llm-key")
				&& !err.contains("fake-stt-key"),
			"round {round}: no secret value in the error: {err}"
		);

		// the ordinary row committed; secrets written before the
		// failing one committed; the failing one and every later
		// one were not attempted
		let after = state.ai_settings();
		assert_eq!(after.stt_language, "fr-FR", "round {round}");
		let expected = [
			after.hf_token.as_str(),
			after.ext_llm_api_key.as_str(),
			after.ext_stt_api_key.as_str(),
		];
		for (index, secret_value) in expected.iter().enumerate() {
			let committed = index < round;
			let stored = if committed {
				vec!["fake-hf-token", "fake-llm-key", "fake-stt-key"][index]
			} else {
				""
			};
			assert_eq!(
				*secret_value, stored,
				"round {round}: secret {index} committed={committed}"
			);
		}
		// and the cache equals what a cold restart loads
		let cold = reopened(dir.path(), &format!("f06-name{round}"));
		assert_eq!(cold.ai_settings().stt_language, "fr-FR");
		assert_eq!(cold.ai_settings().hf_token, after.hf_token);
		assert_eq!(cold.ai_settings().ext_llm_api_key, after.ext_llm_api_key);
		assert_eq!(cold.ai_settings().ext_stt_api_key, after.ext_stt_api_key);
	}
}

#[test]
fn clearing_failure_publishes_actual_stores_and_names_the_secret() {
	let (state, dir) = temp_state("f06-clear");
	let db_path = dir.path().join("f06-clear.db");
	state
		.mutate_ai_settings(|s| {
			s.apply_updates(&serde_json::json!({ "hfToken": "fake-old-token" }))
		})
		.expect("seed token");
	fail_key_deletes(
		&db_path,
		crate::keys::setting::dev_secret::HF_TOKEN,
		"database is locked",
	);

	let err = state
		.mutate_ai_settings(|s| {
			s.apply_updates(&serde_json::json!({
				"sttLanguage": "fr-FR",
				"hfToken": "",
			}))
		})
		.expect_err("the clear fails");
	assert!(
		err.contains("clearing the HuggingFace token failed"),
		"the error must name the failed clear: {err}"
	);

	// ordinary fields committed; the token survived in the store,
	// and the cache says so instead of claiming it was cleared
	assert_eq!(state.ai_settings().stt_language, "fr-FR");
	assert_eq!(state.ai_settings().hf_token, "fake-old-token");
	let cold = reopened(dir.path(), "f06-clear");
	assert_eq!(cold.ai_settings().stt_language, "fr-FR");
	assert_eq!(cold.ai_settings().hf_token, "fake-old-token");
}

#[test]
fn settings_row_failure_keeps_cache_on_actual_stores_and_names_the_stage() {
	// The ordinary row transaction is atomic: nothing commits, so
	// the pre-save state IS the actual state - but the error must
	// still name the settings-row stage, not a secret.
	let (state, dir) = temp_state("f06-row");
	let db_path = dir.path().join("f06-row.db");
	let _ = state.ai_settings();
	fail_key_writes(
		&db_path,
		crate::keys::setting::AI_STT_LANGUAGE,
		"injected row write failure",
	);
	let gen_before = state.ai_settings_generation.load(Ordering::SeqCst);

	let err = state
		.mutate_ai_settings(|s| {
			s.apply_updates(&serde_json::json!({
				"sttLanguage": "fr-FR",
				"extLlmModel": "changed",
			}))
		})
		.expect_err("the settings row transaction fails");
	assert!(
		err.contains("could not save the settings"),
		"the error must name the failing stage: {err}"
	);
	assert!(!err.contains("token"), "no secret is involved: {err}");

	// nothing committed: cache and stores keep the previous values
	assert_eq!(state.ai_settings().stt_language, "en-US");
	assert_eq!(state.ai_settings().ext_llm_model, "");
	assert_eq!(
		reopened(dir.path(), "f06-row").ai_settings().stt_language,
		"en-US"
	);
	assert_eq!(
		state.ai_settings_generation.load(Ordering::SeqCst),
		gen_before,
		"a failed save does not bump the generation"
	);
}

#[test]
fn unreadable_store_after_a_failed_save_drops_the_cache() {
	// The save fails AND the store cannot be read back: the cache
	// must be dropped (degraded mode) rather than left holding or
	// republishing unverified values.
	let (state, dir) = temp_state("f06-degraded");
	let db_path = dir.path().join("f06-degraded.db");
	let _ = state.ai_settings();
	{
		let conn = rusqlite::Connection::open(&db_path).unwrap();
		conn.execute_batch("DROP TABLE settings").unwrap();
	}

	let err = state
		.mutate_ai_settings(|s| s.apply_updates(&serde_json::json!({ "sttLanguage": "fr-FR" })))
		.expect_err("the save fails");
	assert!(
		err.contains("settings state unavailable after a failed save"),
		"degraded mode must be named: {err}"
	);
	assert!(
		state.ai_settings_cache.lock().unwrap().is_none(),
		"a store that cannot be read must force the next read to load cold"
	);
}

#[test]
fn retry_after_partial_commit_preserves_committed_fields_and_retries_the_secret() {
	let (state, dir) = temp_state("f06-retry");
	let db_path = dir.path().join("f06-retry.db");
	let _ = state.ai_settings();
	fail_key_writes(
		&db_path,
		crate::keys::setting::dev_secret::HF_TOKEN,
		"injected secret write failure",
	);

	// the user saves; it fails on the token after the ordinary
	// fields committed
	state
		.mutate_ai_settings(|s| {
			s.apply_updates(&serde_json::json!({
				"sttLanguage": "fr-FR",
				"hfToken": "fake-test-token",
			}))
		})
		.expect_err("first attempt fails on the token");
	drop_fail_key_trigger(&db_path, crate::keys::setting::dev_secret::HF_TOKEN);

	// the retry goes through the boundary again: it must build on
	// the LATEST ACTUAL state, so the committed fr-FR survives and
	// the token (still absent in the store) is re-attempted
	state
		.mutate_ai_settings(|s| {
			s.apply_updates(&serde_json::json!({
				"extLlmModel": "new-model",
				"hfToken": "fake-test-token",
			}))
		})
		.expect("retry commits");

	let after = state.ai_settings();
	assert_eq!(
		after.stt_language, "fr-FR",
		"committed fields are not reverted"
	);
	assert_eq!(after.ext_llm_model, "new-model");
	assert_eq!(after.hf_token, "fake-test-token");
	let cold = reopened(dir.path(), "f06-retry");
	assert_eq!(cold.ai_settings().stt_language, "fr-FR");
	assert_eq!(cold.ai_settings().ext_llm_model, "new-model");
	assert_eq!(cold.ai_settings().hf_token, "fake-test-token");
}
