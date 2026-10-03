//! AI settings: modes, engines, validation, persistence, endpoint
//! resolution.

use super::catalog::{find_model, ModelKind, LLM_MODELS, STT_MODELS};
use crate::db::Db;
use crate::keys::setting;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmMode {
	/// local llama.cpp engine
	Local,
	/// the user-configured OpenAI-compatible endpoint
	External,
}

impl LlmMode {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Local => "local",
			Self::External => "external",
		}
	}
}

impl std::str::FromStr for LlmMode {
	type Err = String;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"local" => Ok(Self::Local),
			"external" => Ok(Self::External),
			_ => Err(format!(
				"invalid llmMode '{s}' (expected local or external)"
			)),
		}
	}
}

/// Where transcription runs. Mirrors [`LlmMode`]: an external endpoint can
/// be saved (and tested) without being used until this says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttMode {
	/// on this computer (Apple Speech or whisper, per `stt_engine`)
	Local,
	/// the user-configured OpenAI-compatible transcription endpoint
	External,
}

impl SttMode {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Local => "local",
			Self::External => "external",
		}
	}
}

impl std::str::FromStr for SttMode {
	type Err = String;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"local" => Ok(Self::Local),
			"external" => Ok(Self::External),
			_ => Err(format!(
				"invalid sttMode '{s}' (expected local or external)"
			)),
		}
	}
}

/// Local speech-to-text engine selection. (Named `SpeechEngine` because
/// `SttEngine` is the loaded whisper engine itself.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechEngine {
	/// Apple Speech on macOS 26+, whisper otherwise
	Auto,
	/// the macOS 26+ built-in engine only
	Apple,
	/// local whisper.cpp only
	Whisper,
}

impl SpeechEngine {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Auto => "auto",
			Self::Apple => "apple",
			Self::Whisper => "whisper",
		}
	}

	/// The engine that actually handles local transcription for this
	/// choice: Apple when explicitly selected or when auto + available,
	/// whisper otherwise.
	pub fn effective(self) -> Self {
		self.effective_with(crate::apple::speech_available())
	}

	/// [`Self::effective`] for a given Apple Speech availability (pure).
	pub fn effective_with(self, apple_available: bool) -> Self {
		match self {
			Self::Apple => Self::Apple,
			Self::Auto if apple_available => Self::Apple,
			_ => Self::Whisper,
		}
	}
}

impl std::str::FromStr for SpeechEngine {
	type Err = String;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"auto" => Ok(Self::Auto),
			"apple" => Ok(Self::Apple),
			"whisper" => Ok(Self::Whisper),
			_ => Err(format!(
				"invalid sttEngine '{s}' (expected auto, apple, or whisper)"
			)),
		}
	}
}

/// AI-related settings resolved from the settings table.
#[derive(Debug, Clone)]
pub struct AiSettings {
	pub llm_mode: LlmMode,
	pub llm_model: String,
	pub stt_model: String,
	/// Local or external transcription (see [`Self::uses_external_stt`]).
	pub stt_mode: SttMode,
	/// Speech-to-text engine selection.
	pub stt_engine: SpeechEngine,
	/// BCP-47 locale for the Apple Speech engine (e.g. "en-US").
	pub stt_language: String,
	/// HuggingFace access token; sent with model downloads, where it
	/// avoids anonymous rate limits and can speed up large transfers.
	pub hf_token: String,
	/// HuggingFace download endpoint override (mirror). Empty falls back
	/// to the HF_ENDPOINT environment variable, then huggingface.co. A
	/// GUI app does not inherit shell env vars, so this setting is how
	/// mirror users configure one without launchctl gymnastics.
	pub hf_endpoint: String,
	pub ext_llm_base_url: String,
	pub ext_llm_api_key: String,
	pub ext_llm_model: String,
	pub ext_stt_base_url: String,
	pub ext_stt_api_key: String,
	pub ext_stt_model: String,
}

/// True when `url` starts with an http:// or https:// scheme - the one
/// check every configurable endpoint URL (external LLM/STT, HF mirror)
/// goes through, at save time and again before use.
pub fn has_http_scheme(url: &str) -> bool {
	url.starts_with("http://") || url.starts_with("https://")
}

/// Engine default for the first launch after this setting was introduced:
/// installs that already have AI configuration keep whisper (no behavior
/// change), brand-new installs get "auto". Pure - no writes; setup
/// persists the default once so this read-path helper never mutates.
pub fn default_stt_engine(db: &Db) -> SpeechEngine {
	const PREVIOUS_AI_KEYS: [&str; 4] = [
		setting::AI_LLM_MODE,
		setting::AI_LLM_MODEL,
		setting::AI_STT_MODEL,
		setting::EXT_STT_BASE_URL,
	];
	let existing_install = PREVIOUS_AI_KEYS.iter().any(|k| db.get_setting(k).is_some());
	if existing_install {
		SpeechEngine::Whisper
	} else {
		SpeechEngine::Auto
	}
}

impl AiSettings {
	pub fn load(db: &Db) -> Self {
		let get = |k: &str| db.get_setting(k).unwrap_or_default();
		let secret = |s: crate::secrets::Secret| crate::secrets::load(s, db).unwrap_or_default();
		Self {
			llm_mode: {
				// Only two modes exist; an unknown stored value (legacy or
				// hand-edited) degrades to local - the same rule generation
				// applies, so the loader's status can never disagree with
				// what chats actually use.
				get(setting::AI_LLM_MODE)
					.parse::<LlmMode>()
					.unwrap_or(LlmMode::Local)
			},
			llm_model: {
				let m = get(setting::AI_LLM_MODEL);
				if m.is_empty() {
					LLM_MODELS[0].id.to_string()
				} else {
					m
				}
			},
			stt_model: {
				let m = get(setting::AI_STT_MODEL);
				if m.is_empty() {
					STT_MODELS[0].id.to_string()
				} else {
					m
				}
			},
			stt_mode: {
				// Before this setting existed a saved STT URL alone meant
				// "external", so an install without an explicit choice keeps
				// that behaviour; an unknown stored value is treated the same.
				let legacy = if get(setting::EXT_STT_BASE_URL).is_empty() {
					SttMode::Local
				} else {
					SttMode::External
				};
				get(setting::AI_STT_MODE)
					.parse::<SttMode>()
					.unwrap_or(legacy)
			},
			stt_engine: match db.get_setting(setting::AI_STT_ENGINE) {
				Some(v) => v
					.parse::<SpeechEngine>()
					.unwrap_or_else(|_| default_stt_engine(db)),
				None => default_stt_engine(db),
			},
			stt_language: {
				let v = get(setting::AI_STT_LANGUAGE);
				if v.is_empty() {
					"en-US".into()
				} else {
					v
				}
			},
			hf_token: secret(crate::secrets::Secret::HfToken),
			hf_endpoint: get(setting::HF_ENDPOINT),
			ext_llm_base_url: get(setting::EXT_LLM_BASE_URL),
			ext_llm_api_key: secret(crate::secrets::Secret::ExtLlmApiKey),
			ext_llm_model: get(setting::EXT_LLM_MODEL),
			ext_stt_base_url: get(setting::EXT_STT_BASE_URL),
			ext_stt_api_key: secret(crate::secrets::Secret::ExtSttApiKey),
			ext_stt_model: get(setting::EXT_STT_MODEL),
		}
	}

	/// The engine that handles local transcription for the current
	/// choice (Apple when selected/available, whisper otherwise).
	pub fn effective_stt_engine(&self) -> SpeechEngine {
		self.stt_engine.effective()
	}

	/// True when chat generation should use the external OpenAI-compatible
	/// endpoint. Single source of truth for the mode check: anything but
	/// External means the local engine, so callers can never disagree
	/// about which backend a setting routes to.
	pub fn uses_external_llm(&self) -> bool {
		self.llm_mode == LlmMode::External
	}

	/// True when transcription should go to the external endpoint: the
	/// mode says so and a URL is saved. Without a URL there is nothing to
	/// call, so it falls back to local rather than failing every recording.
	/// Single source of truth for the routing decision.
	pub fn uses_external_stt(&self) -> bool {
		self.stt_mode == SttMode::External && !self.ext_stt_base_url.is_empty()
	}

	/// Validate and apply a partial update from the settings form
	/// (absent/null fields keep their value). Err rejects the whole
	/// update; nothing is applied on failure.
	pub fn apply_updates(&mut self, ai: &serde_json::Value) -> Result<(), String> {
		let get_str = |key: &str| ai[key].as_str().map(|s| s.to_string());

		if let Some(v) = get_str("llmMode") {
			self.llm_mode = v.parse::<LlmMode>()?;
		}
		if let Some(v) = get_str("llmModel") {
			if find_model(&v, ModelKind::Llm).is_none() {
				return Err(format!("unknown llmModel '{v}'"));
			}
			self.llm_model = v;
		}
		if let Some(v) = get_str("sttModel") {
			if find_model(&v, ModelKind::Stt).is_none() {
				return Err(format!("unknown sttModel '{v}'"));
			}
			self.stt_model = v;
		}
		if let Some(v) = get_str("sttMode") {
			self.stt_mode = v.parse::<SttMode>()?;
		}
		if let Some(v) = get_str("sttEngine") {
			self.stt_engine = v.parse::<SpeechEngine>()?;
		}
		if let Some(v) = get_str("sttLanguage") {
			// BCP-47-ish locale id ("en-US"); short, letters/digits/hyphen only.
			let cleaned = v.trim();
			if !cleaned.is_empty() {
				let valid = cleaned.len() <= 16
					&& cleaned
						.chars()
						.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
				if !valid {
					return Err(format!(
						"invalid sttLanguage '{cleaned}' (expected a locale like en-US)"
					));
				}
				self.stt_language = cleaned.to_string();
			}
		}
		let base_url = |field: &str, v: &str| -> Result<(), String> {
			if v.is_empty() || has_http_scheme(v) {
				Ok(())
			} else {
				Err(format!(
					"invalid {field} '{v}' (include http:// or https://)"
				))
			}
		};
		if let Some(v) = get_str("extLlmBaseUrl") {
			base_url("extLlmBaseUrl", &v)?;
			self.ext_llm_base_url = v;
		}
		if let Some(v) = get_str("extLlmModel") {
			self.ext_llm_model = v;
		}
		if let Some(v) = get_str("extSttBaseUrl") {
			base_url("extSttBaseUrl", &v)?;
			self.ext_stt_base_url = v;
		}
		if let Some(v) = get_str("extSttModel") {
			self.ext_stt_model = v;
		}
		// Secrets: the real value never comes back to the webview, so an
		// absent/null field keeps the stored value and an explicit "" clears it.
		if let Some(v) = get_str("hfEndpoint") {
			base_url("hfEndpoint", &v)?;
			self.hf_endpoint = v.trim().trim_end_matches('/').to_string();
		}
		if let Some(v) = get_str("hfToken") {
			self.hf_token = v;
		}
		if let Some(v) = get_str("extLlmApiKey") {
			self.ext_llm_api_key = v;
		}
		if let Some(v) = get_str("extSttApiKey") {
			self.ext_stt_api_key = v;
		}
		Ok(())
	}

	/// Settings-table key nothing ever writes: deleting it is a no-op
	/// on a healthy store, and fails exactly when the settings table
	/// cannot serve a statement at all. `Db`'s read API deliberately
	/// swallows read errors (a broken table reads as empty), so this
	/// no-op delete is the only way [`AiSettings::reload`] can tell
	/// "unreadable" apart from "factory defaults".
	const STORE_HEALTH_KEY: &str = "settings_store_health_probe";

	/// Persist these settings in two stages: the ordinary settings
	/// row in one transaction (stage 1), then each changed secret
	/// sequentially (stage 2). `previous` is what is stored now (the
	/// AppState cache, or a fresh load): secrets equal to it are left
	/// alone, so a save does no keychain round-trips (each is a
	/// syscall and can trigger a macOS permission prompt) for secrets
	/// the user did not touch.
	///
	/// Err records the failing stage ([`SaveFailure`]). SQLite cannot
	/// transact the OS keychain, so the stages cannot be atomic: a
	/// [`SaveStage::Secret`] failure means the settings row - and
	/// possibly secrets earlier in the sequence - IS committed while
	/// Err is returned. Callers must reconcile with the actual stores
	/// ([`Self::reload`]) instead of reporting success or pretending
	/// the row transaction rolled the keychain back.
	pub fn save(&self, db: &Db, previous: &AiSettings) -> Result<(), SaveFailure> {
		db.set_settings(&[
			(setting::AI_LLM_MODE, self.llm_mode.as_str().to_string()),
			(setting::AI_LLM_MODEL, self.llm_model.clone()),
			(setting::AI_STT_MODEL, self.stt_model.clone()),
			(setting::AI_STT_MODE, self.stt_mode.as_str().to_string()),
			(setting::AI_STT_ENGINE, self.stt_engine.as_str().to_string()),
			(setting::AI_STT_LANGUAGE, self.stt_language.clone()),
			(setting::HF_ENDPOINT, self.hf_endpoint.clone()),
			(setting::EXT_LLM_BASE_URL, self.ext_llm_base_url.clone()),
			(setting::EXT_LLM_MODEL, self.ext_llm_model.clone()),
			(setting::EXT_STT_BASE_URL, self.ext_stt_base_url.clone()),
			(setting::EXT_STT_MODEL, self.ext_stt_model.clone()),
		])
		.map_err(|error| SaveFailure {
			stage: SaveStage::SettingsRow,
			error,
		})?;
		let save_secret = |secret: crate::secrets::Secret,
		                   value: &str,
		                   stored: &str|
		 -> Result<(), SaveFailure> {
			if value == stored {
				return Ok(());
			}
			let clearing = value.is_empty();
			let result = if clearing {
				crate::secrets::clear(secret, db)
			} else {
				crate::secrets::store(secret, value, db)
			};
			result.map_err(|error| SaveFailure {
				stage: SaveStage::Secret { secret, clearing },
				error,
			})
		};
		save_secret(
			crate::secrets::Secret::HfToken,
			&self.hf_token,
			&previous.hf_token,
		)?;
		save_secret(
			crate::secrets::Secret::ExtLlmApiKey,
			&self.ext_llm_api_key,
			&previous.ext_llm_api_key,
		)?;
		save_secret(
			crate::secrets::Secret::ExtSttApiKey,
			&self.ext_stt_api_key,
			&previous.ext_stt_api_key,
		)?;
		Ok(())
	}

	/// The authoritative state RIGHT NOW: the DB rows plus secret
	/// stores as they are actually readable. For reconciling after a
	/// failed save - unlike [`Self::load`], Err means the backing
	/// store could not be read at all, and the caller must drop its
	/// cache (degraded mode: the next read loads cold) rather than
	/// republish values that mask a broken store.
	pub fn reload(db: &Db) -> Result<Self, String> {
		db.delete_setting(Self::STORE_HEALTH_KEY)
			.map_err(|e| format!("settings store unreadable: {e}"))?;
		Ok(Self::load(db))
	}
}

/// Which stage of a staged [`AiSettings::save`] failed. Internal to
/// the settings persistence boundary - it never crosses IPC, but its
/// text is what user-facing errors are built from, so it names the
/// stage/secret without ever including a secret value. Lets callers
/// tell the three persistence outcomes apart:
///
/// - [`SaveStage::SettingsRow`] - the one row transaction failed
///   atomically, so nothing committed (the state is unchanged);
/// - [`SaveStage::Secret`] - the row and possibly earlier secrets
///   committed; the named secret did not (partially committed);
/// - a failed [`AiSettings::reload`] afterwards - reconciliation
///   itself failed; the settings state is unknown (degraded mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStage {
	SettingsRow,
	/// Records which secret failed and whether the failed operation
	/// was clearing it (`clearing: true`) or storing a new value.
	Secret {
		secret: crate::secrets::Secret,
		clearing: bool,
	},
}

/// A staged save failure: the stage that failed plus the store's own
/// error text (row keys and store status - never a secret value).
#[derive(Debug, Clone)]
pub struct SaveFailure {
	pub stage: SaveStage,
	pub error: String,
}

/// The settings-form name of a secret, so errors read like the form
/// ("the HuggingFace token") instead of a store row key, and never
/// carry the value.
fn secret_name(secret: crate::secrets::Secret) -> &'static str {
	match secret {
		crate::secrets::Secret::HfToken => "HuggingFace token",
		crate::secrets::Secret::ExtLlmApiKey => "external LLM API key",
		crate::secrets::Secret::ExtSttApiKey => "external STT API key",
	}
}

impl std::fmt::Display for SaveFailure {
	/// Names the failing stage/secret and, for secret failures, that
	/// the ordinary settings DID save - the honest partial outcome.
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self.stage {
			SaveStage::SettingsRow => {
				write!(f, "could not save the settings: {}", self.error)
			}
			SaveStage::Secret { secret, clearing } => write!(
				f,
				"saved settings, but {} the {} failed: {}",
				if clearing { "clearing" } else { "storing" },
				secret_name(secret),
				self.error
			),
		}
	}
}

pub fn resolve_hf_endpoint(setting: &str, env: Option<&str>) -> String {
	let clean = |v: &str| v.trim().trim_end_matches('/').to_string();
	if !setting.trim().is_empty() {
		return clean(setting);
	}
	env.map(clean)
		.filter(|e| !e.is_empty())
		.unwrap_or_else(|| "https://huggingface.co".into())
}

/// The endpoint this install downloads models from, resolving the
/// in-app setting against the environment.
pub fn hf_endpoint(settings: &AiSettings) -> String {
	resolve_hf_endpoint(
		&settings.hf_endpoint,
		std::env::var("HF_ENDPOINT").ok().as_deref(),
	)
}

#[cfg(test)]
mod tests {
	use super::AiSettings;
	use crate::db::Db;
	use crate::keys::setting;

	fn temp_db(name: &str) -> (Db, tempfile::TempDir) {
		// TempDir keeps the file (and its WAL sidecars) alive until the
		// test ends and removes them all together - deleting the DB file
		// while the connection was open failed on Windows.
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join(format!("{name}.db"))).expect("open test db");
		(db, dir)
	}

	#[test]
	fn stt_engine_defaults_to_auto_for_new_installs() {
		let (db, _dir) = temp_db("fresh");
		let s = AiSettings::load(&db);
		assert_eq!(s.stt_engine, super::SpeechEngine::Auto);
		// loading is a pure read now: the default is persisted once by
		// app setup, not as a side effect of every load
		assert_eq!(
			db.get_setting(setting::AI_STT_ENGINE),
			None,
			"load must not write the resolved default"
		);
	}

	#[test]
	fn stt_engine_defaults_to_whisper_for_existing_installs() {
		let (db, _dir) = temp_db("existing");
		db.set_setting(setting::AI_STT_MODEL, "whisper-small-en")
			.expect("set");
		let s = AiSettings::load(&db);
		assert_eq!(s.stt_engine, super::SpeechEngine::Whisper);
	}

	#[test]
	fn stt_engine_keeps_stored_value() {
		let (db, _dir) = temp_db("stored");
		db.set_setting(setting::AI_STT_ENGINE, "apple")
			.expect("set");
		let s = AiSettings::load(&db);
		assert_eq!(s.stt_engine, super::SpeechEngine::Apple);
	}

	#[test]
	fn stt_language_defaults_to_en_us_and_round_trips() {
		let (db, _dir) = temp_db("lang");
		let mut s = AiSettings::load(&db);
		let previous = s.clone();
		assert_eq!(s.stt_language, "en-US");
		s.stt_language = "de-DE".into();
		s.save(&db, &previous).expect("save");
		assert_eq!(AiSettings::load(&db).stt_language, "de-DE");
	}

	#[test]
	fn save_reports_database_failures() {
		let dir = tempfile::tempdir().expect("tempdir");
		let path = dir.path().join("save-fail.db");
		let db = Db::open(&path).expect("open");
		{
			// break the settings table behind Db's back so the write
			// fails (simulates a full/locked database)
			let conn = rusqlite::Connection::open(&path).unwrap();
			conn.execute_batch("DROP TABLE settings").unwrap();
		}
		let loaded = AiSettings::load(&db);
		let failure = loaded
			.save(&db, &loaded)
			.expect_err("save must surface the failure instead of logging it");
		// the stage is structured, so callers can tell "nothing
		// committed" (row transaction) from "partially committed"
		// (secret stage) without parsing text
		assert_eq!(failure.stage, super::SaveStage::SettingsRow);
		let err = failure.to_string();
		assert!(
			err.contains("could not save the settings"),
			"the stage is named: {err}"
		);
		assert!(
			err.contains("failed to save setting"),
			"the store's own error surfaces: {err}"
		);
	}

	#[test]
	fn clearing_a_secret_that_cannot_be_removed_fails_the_save() {
		// DB-fallback path only: the row delete fails before any keychain
		// call, so this never touches the developer's real keychain.
		let dir = tempfile::tempdir().expect("tempdir");
		let path = dir.path().join("clear-fail.db");
		let db = Db::open(&path).expect("open");
		// the row this build keeps the token in (dev-only in debug builds)
		let row = crate::secrets::Secret::HfToken.store_key();
		db.set_setting(row, "hf_old").expect("seed fallback row");
		{
			let conn = rusqlite::Connection::open(&path).unwrap();
			conn.execute_batch(&format!(
				"CREATE TRIGGER keep_token BEFORE DELETE ON settings
				 WHEN OLD.key = '{row}'
				 BEGIN SELECT RAISE(ABORT, 'database is locked'); END;"
			))
			.unwrap();
		}
		let previous = AiSettings::load(&db);
		let mut settings = previous.clone();
		settings.hf_token = String::new();
		let failure = settings
			.save(&db, &previous)
			.expect_err("a secret that survives a clear must not report success");
		assert_eq!(
			failure.stage,
			super::SaveStage::Secret {
				secret: crate::secrets::Secret::HfToken,
				clearing: true,
			}
		);
		let err = failure.to_string();
		assert!(
			err.contains("clearing the HuggingFace token failed"),
			"the failed secret is named: {err}"
		);
		assert!(err.contains("database is locked"), "unexpected: {err}");
		assert!(
			!err.contains("hf_old"),
			"no secret value in the error: {err}"
		);
		assert_eq!(db.get_setting(row).as_deref(), Some("hf_old"));
	}

	#[test]
	fn reload_tells_unreadable_stores_from_empty_ones() {
		let (db, _dir) = temp_db("reload-ok");
		// a healthy store reloads the same state load() reports
		db.set_setting(setting::AI_STT_LANGUAGE, "fr-FR")
			.expect("set");
		let reloaded = AiSettings::reload(&db).expect("healthy store reloads");
		assert_eq!(reloaded.stt_language, "fr-FR");
		// and the health probe leaves no row behind
		assert_eq!(db.get_setting(super::AiSettings::STORE_HEALTH_KEY), None);

		// a store whose settings table cannot serve a statement at all
		// must Err: swallowing that would republish factory defaults
		// that mask a broken store
		let dir = tempfile::tempdir().expect("tempdir");
		let path = dir.path().join("reload-fail.db");
		let broken = Db::open(&path).expect("open");
		{
			let conn = rusqlite::Connection::open(&path).unwrap();
			conn.execute_batch("DROP TABLE settings").unwrap();
		}
		let err = AiSettings::reload(&broken)
			.expect_err("an unreadable store must not masquerade as defaults");
		assert!(
			err.contains("settings store unreadable"),
			"unexpected: {err}"
		);
	}

	#[test]
	fn effective_engine_matches_availability() {
		let (db, _dir) = temp_db("effective");
		let mut s = AiSettings::load(&db);
		s.stt_engine = super::SpeechEngine::Apple;
		assert_eq!(s.effective_stt_engine(), super::SpeechEngine::Apple);
		s.stt_engine = super::SpeechEngine::Whisper;
		assert_eq!(s.effective_stt_engine(), super::SpeechEngine::Whisper);
		s.stt_engine = super::SpeechEngine::Auto;
		let expected = if crate::apple::speech_available() {
			super::SpeechEngine::Apple
		} else {
			super::SpeechEngine::Whisper
		};
		assert_eq!(s.effective_stt_engine(), expected);
	}

	#[test]
	fn mode_and_engine_parsing_reject_unknown_values() {
		use super::{LlmMode, SpeechEngine};
		assert_eq!("local".parse::<LlmMode>().unwrap(), LlmMode::Local);
		assert_eq!("external".parse::<LlmMode>().unwrap(), LlmMode::External);
		assert!("banana".parse::<LlmMode>().is_err());
		for raw in ["auto", "apple", "whisper"] {
			raw.parse::<SpeechEngine>().expect(raw);
		}
		assert!("sometimes".parse::<SpeechEngine>().is_err());
	}

	#[test]
	fn unknown_llm_mode_degrades_to_local_everywhere() {
		let (db, _dir) = temp_db("mode");
		// a garbage stored mode (legacy/hand-edited row) must not split the
		// callers: generation and the model loader agree on local
		db.set_setting(setting::AI_LLM_MODE, "banana").expect("set");
		let s = AiSettings::load(&db);
		assert_eq!(s.llm_mode, super::LlmMode::Local);
		assert!(!s.uses_external_llm());
		db.set_setting(setting::AI_LLM_MODE, "external")
			.expect("set");
		assert!(AiSettings::load(&db).uses_external_llm());
		db.set_setting(setting::AI_LLM_MODE, "local").expect("set");
		assert!(!AiSettings::load(&db).uses_external_llm());
	}

	#[test]
	fn apply_updates_validates_and_applies_atomically() {
		let (db, _dir) = temp_db("apply");
		let mut s = AiSettings::load(&db);
		s.apply_updates(&serde_json::json!({
			"llmMode": "external",
			"llmModel": "gemma-4-E4B",
			"sttModel": "whisper-small-en",
			"extLlmBaseUrl": "http://localhost:1234/v1",
		}))
		.expect("valid update applies");
		assert!(s.uses_external_llm());
		assert_eq!(s.llm_model, "gemma-4-E4B");
		assert_eq!(s.stt_model, "whisper-small-en");

		let mut bad = AiSettings::load(&db);
		for field in [
			serde_json::json!({ "llmMode": "sometimes" }),
			serde_json::json!({ "llmModel": "not-a-model" }),
			serde_json::json!({ "sttModel": "gemma-4-E2B-qat" }), // llm id, wrong catalog
			serde_json::json!({ "extLlmBaseUrl": "localhost:1234" }), // no scheme
			serde_json::json!({ "extSttBaseUrl": "ftp://example.com" }),
		] {
			assert!(
				bad.apply_updates(&field).is_err(),
				"update must be rejected: {field}"
			);
		}
		// nothing from a rejected update leaked into the settings
		assert_eq!(bad.llm_mode, super::LlmMode::Local);
		assert_eq!(bad.ext_llm_base_url, "");
		// an empty base URL is fine (the endpoint is simply unused)
		bad.apply_updates(&serde_json::json!({ "extLlmBaseUrl": "" }))
			.expect("empty base url allowed");
	}

	#[test]
	fn stt_mode_defaults_keep_existing_external_stt_users_external() {
		// fresh install: local
		let (db, _dir) = temp_db("stt-mode-fresh");
		assert_eq!(AiSettings::load(&db).stt_mode, super::SttMode::Local);

		// before stt_mode existed, a saved URL alone meant "external" -
		// such installs must keep transcribing externally
		let (db, _dir) = temp_db("stt-mode-legacy");
		db.set_setting(setting::EXT_STT_BASE_URL, "http://localhost:8080")
			.unwrap();
		let s = AiSettings::load(&db);
		assert_eq!(s.stt_mode, super::SttMode::External);
		assert!(s.uses_external_stt());

		// an explicit choice wins over the URL
		db.set_setting(setting::AI_STT_MODE, "local").unwrap();
		let s = AiSettings::load(&db);
		assert_eq!(s.stt_mode, super::SttMode::Local);
		assert!(
			!s.uses_external_stt(),
			"a saved URL alone no longer routes STT"
		);
	}

	#[test]
	fn external_stt_needs_both_the_mode_and_a_url() {
		let (db, _dir) = temp_db("stt-mode-url");
		let mut s = AiSettings::load(&db);
		s.stt_mode = super::SttMode::External;
		s.ext_stt_base_url = String::new();
		assert!(
			!s.uses_external_stt(),
			"no URL: fall back to local, don't error"
		);
		s.ext_stt_base_url = "http://localhost:8080".into();
		assert!(s.uses_external_stt());
	}

	#[test]
	fn stt_mode_is_validated_and_persisted() {
		let (db, _dir) = temp_db("stt-mode-save");
		let mut s = AiSettings::load(&db);
		assert!(s
			.apply_updates(&serde_json::json!({ "sttMode": "cloud" }))
			.is_err());
		s.apply_updates(&serde_json::json!({ "sttMode": "external" }))
			.expect("valid mode");
		let previous = AiSettings::load(&db);
		s.save(&db, &previous).expect("save");
		assert_eq!(
			db.get_setting(setting::AI_STT_MODE).as_deref(),
			Some("external")
		);
		assert_eq!(AiSettings::load(&db).stt_mode, super::SttMode::External);
	}
}
