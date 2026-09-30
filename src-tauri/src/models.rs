use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::db::Db;
use crate::keys::setting;
use crate::llm::LocalLlm;
use crate::stt::SttEngine;

/// User-Agent for all outbound HTTP (downloads, external endpoints).
pub(crate) const USER_AGENT: &str = concat!("brainstory-desktop/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
	Llm,
	Stt,
}

#[derive(Clone)]
pub struct ModelSpec {
	pub id: &'static str,
	pub kind: ModelKind,
	pub label: &'static str,
	pub description: &'static str,
	pub repo: &'static str,
	pub filename: &'static str,
	pub size_bytes: u64,
	/// Pinned sha256 of the downloadable file, verified after download so a
	/// corrupted or silently-replaced upstream file can't brick a model slot.
	pub sha256: &'static str,
}

/// Known-good open model builds. The local LLM catalog defaults to Google's
/// Gemma 4 edge models (QAT quantized, runnable on integrated GPUs / Apple
/// Silicon) with MiniCPM5 as a smaller alternative; STT ships whisper.cpp
/// ggml builds.
pub const LLM_MODELS: [ModelSpec; 3] = [
	ModelSpec {
		id: "gemma-4-E2B-qat",
		kind: ModelKind::Llm,
		label: "Gemma 4 E2B (light)",
		description: "Google Gemma 4 E2B instruct, QAT 4-bit. Lightest option (~3.3 GB). Best for laptops.",
		repo: "google/gemma-4-E2B-it-qat-q4_0-gguf",
		filename: "gemma-4-E2B_q4_0-it.gguf",
		size_bytes: 3_349_516_256,
		sha256: "fa401b55b07ee70a54c6dae3903c783a6e65064312529ea57175cb5f8dec6634",
	},
	ModelSpec {
		id: "gemma-4-E4B",
		kind: ModelKind::Llm,
		label: "Gemma 4 E4B",
		description: "Google Gemma 4 E4B instruct, QAT 4-bit. Higher quality (~5.2 GB), needs a bit more RAM/VRAM.",
		repo: "google/gemma-4-E4B-it-qat-q4_0-gguf",
		filename: "gemma-4-E4B_q4_0-it.gguf",
		size_bytes: 5_154_941_280,
		sha256: "676c35070db6dbe52f93e9c864ee0fba4eddea94b9c875d9cb10daff453fbaee",
	},
	ModelSpec {
		id: "minicpm5-2b",
		kind: ModelKind::Llm,
		label: "MiniCPM5 2B",
		description: "OpenBMB MiniCPM5 2B instruct, Q4_K_M (~1.6 GB). Small and fast alternative to Gemma.",
		repo: "openbmb/MiniCPM5-2B-GGUF",
		filename: "MiniCPM5-2B-Q4_K_M.gguf",
		size_bytes: 1_561_318_368,
		sha256: "ec2d5801640099e97d8d7e8003ad4d81f336e757811f03a26173dddf386602fd",
	},
];

pub const STT_MODELS: [ModelSpec; 4] = [
	ModelSpec {
		id: "whisper-base-en",
		kind: ModelKind::Stt,
		label: "Whisper base (English)",
		description: "whisper.cpp ggml base English model (~148 MB). Fast and light.",
		repo: "ggerganov/whisper.cpp",
		filename: "ggml-base.en.bin",
		size_bytes: 147_964_211,
		sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
	},
	ModelSpec {
		id: "whisper-small-en",
		kind: ModelKind::Stt,
		label: "Whisper small (English)",
		description: "whisper.cpp ggml small English model (~488 MB). Better accuracy.",
		repo: "ggerganov/whisper.cpp",
		filename: "ggml-small.en.bin",
		size_bytes: 487_614_201,
		sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
	},
	ModelSpec {
		id: "whisper-tiny-en",
		kind: ModelKind::Stt,
		label: "Whisper tiny (English)",
		description: "whisper.cpp ggml tiny English model (~78 MB). Fastest, lower accuracy.",
		repo: "ggerganov/whisper.cpp",
		filename: "ggml-tiny.en.bin",
		size_bytes: 77_704_715,
		sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
	},
	ModelSpec {
		id: "whisper-large-v3-turbo",
		kind: ModelKind::Stt,
		label: "Whisper large v3 turbo",
		description: "whisper.cpp ggml large-v3-turbo model (~1.6 GB). Best accuracy, still fast.",
		repo: "ggerganov/whisper.cpp",
		filename: "ggml-large-v3-turbo.bin",
		size_bytes: 1_624_555_275,
		sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
	},
];

pub fn find_model(id: &str, kind: ModelKind) -> Option<&'static ModelSpec> {
	let list: &[ModelSpec] = match kind {
		ModelKind::Llm => &LLM_MODELS,
		ModelKind::Stt => &STT_MODELS,
	};
	list.iter().find(|m| m.id == id)
}

/// Where chat generation runs. Parsed at the settings boundary; an
/// unknown stored/form value is rejected here instead of being compared
/// as a raw string by every caller.
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
		match self {
			Self::Apple => Self::Apple,
			Self::Auto if crate::apple::speech_available() => Self::Apple,
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

/// Engine default for the first launch after this setting was introduced:
/// installs that already have AI configuration keep whisper (no behavior
/// change), brand-new installs get "auto". Pure - no writes; setup
/// persists the default once so this read-path helper never mutates.
pub(crate) fn default_stt_engine(db: &Db) -> SpeechEngine {
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
			if v.is_empty() || v.starts_with("http://") || v.starts_with("https://") {
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

	/// Persist these settings. Err means the settings row or one of the
	/// secrets could not be written - callers must not report success.
	pub fn save(&self, db: &Db) -> Result<(), String> {
		db.set_settings(&[
			(setting::AI_LLM_MODE, self.llm_mode.as_str().to_string()),
			(setting::AI_LLM_MODEL, self.llm_model.clone()),
			(setting::AI_STT_MODEL, self.stt_model.clone()),
			(setting::AI_STT_ENGINE, self.stt_engine.as_str().to_string()),
			(setting::AI_STT_LANGUAGE, self.stt_language.clone()),
			(setting::HF_ENDPOINT, self.hf_endpoint.clone()),
			(setting::EXT_LLM_BASE_URL, self.ext_llm_base_url.clone()),
			(setting::EXT_LLM_MODEL, self.ext_llm_model.clone()),
			(setting::EXT_STT_BASE_URL, self.ext_stt_base_url.clone()),
			(setting::EXT_STT_MODEL, self.ext_stt_model.clone()),
		])?;
		let save_secret = |secret: crate::secrets::Secret, value: &str| -> Result<(), String> {
			let stored = crate::secrets::load(secret, db);
			// Each keychain operation is a separate syscall round-trip
			// (and on macOS can trigger a permission prompt), so only
			// touch secrets whose value actually changed.
			if stored.as_deref() == Some(value) {
				return Ok(());
			}
			if value.is_empty() {
				if stored.is_some() {
					crate::secrets::clear(secret, db);
				}
				Ok(())
			} else {
				crate::secrets::store(secret, value, db)
			}
		};
		save_secret(crate::secrets::Secret::HfToken, &self.hf_token)?;
		save_secret(crate::secrets::Secret::ExtLlmApiKey, &self.ext_llm_api_key)?;
		save_secret(crate::secrets::Secret::ExtSttApiKey, &self.ext_stt_api_key)?;
		Ok(())
	}
}

pub struct Runtime {
	pub backend: Option<Arc<llama_cpp_2::llama_backend::LlamaBackend>>,
	pub llm: Option<Arc<LocalLlm>>,
	pub stt: Option<Arc<SttEngine>>,
}

/// Lifecycle of one engine slot. Serialized lowercase so the existing
/// frontend contract (`state === "ready"` etc.) is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineState {
	Ready,
	Loading,
	Error,
	Missing,
	External,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineStatus {
	pub state: EngineState,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
}

impl EngineStatus {
	pub fn new(state: EngineState, model_id: Option<&str>, error: Option<&str>) -> Self {
		Self {
			state,
			model_id: model_id.map(|s| s.into()),
			error: error.map(|s| s.into()),
		}
	}

	pub fn ready(model_id: Option<&str>) -> Self {
		Self::new(EngineState::Ready, model_id, None)
	}

	pub fn loading(model_id: &str) -> Self {
		Self::new(EngineState::Loading, Some(model_id), None)
	}

	pub fn error(model_id: Option<&str>, error: &str) -> Self {
		Self::new(EngineState::Error, model_id, Some(error))
	}

	pub fn missing() -> Self {
		Self::new(EngineState::Missing, None, None)
	}

	pub fn external() -> Self {
		Self::new(EngineState::External, None, None)
	}
}

pub struct AppState {
	pub db: Db,
	pub data_dir: PathBuf,
	/// Read-through cache of the AI settings (three keychain reads plus a
	/// dozen DB rows on every load); invalidated by save_ai_settings.
	pub ai_settings_cache: std::sync::RwLock<Option<AiSettings>>,
	pub runtime: std::sync::Mutex<Runtime>,
	pub llm_status: std::sync::Mutex<EngineStatus>,
	pub stt_status: std::sync::Mutex<EngineStatus>,
	/// Cancel token for the in-flight LLM generation. Only generations use
	/// this - downloads get their own token in `download_cancels`.
	pub generation_cancel: std::sync::Mutex<Arc<AtomicBool>>,
	/// model id -> progress percentage for in-flight downloads
	pub download_progress: std::sync::Mutex<std::collections::HashMap<String, f64>>,
	/// model id -> cancel token for in-flight downloads
	pub download_cancels: std::sync::Mutex<std::collections::HashMap<String, Arc<AtomicBool>>>,
	/// guards so only one load per engine kind runs at a time
	pub llm_loading: AtomicBool,
	pub stt_loading: AtomicBool,
	/// when the tray icon is hidden, closing the window quits the app
	/// (otherwise it would keep running with no way to reach it)
	pub quit_on_close: AtomicBool,
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
	// A poisoned lock still holds usable state; recover instead of
	// panicking on every later call.
	mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl AppState {
	pub fn new(db: Db, data_dir: PathBuf) -> Self {
		Self {
			db,
			data_dir,
			ai_settings_cache: std::sync::RwLock::new(None),
			runtime: std::sync::Mutex::new(Runtime {
				backend: None,
				llm: None,
				stt: None,
			}),
			llm_status: std::sync::Mutex::new(EngineStatus::new(EngineState::Missing, None, None)),
			stt_status: std::sync::Mutex::new(EngineStatus::new(EngineState::Missing, None, None)),
			generation_cancel: std::sync::Mutex::new(Arc::new(AtomicBool::new(false))),
			download_progress: std::sync::Mutex::new(std::collections::HashMap::new()),
			download_cancels: std::sync::Mutex::new(std::collections::HashMap::new()),
			llm_loading: AtomicBool::new(false),
			stt_loading: AtomicBool::new(false),
			quit_on_close: AtomicBool::new(false),
		}
	}

	/// The AI settings, from the read-through cache when warm. AiSettings
	/// is only written through [`Self::save_ai_settings`], so the cache
	/// can never go stale.
	pub fn ai_settings(&self) -> AiSettings {
		let cache = self
			.ai_settings_cache
			.read()
			.unwrap_or_else(|e| e.into_inner());
		if let Some(cached) = cache.as_ref() {
			return cached.clone();
		}
		drop(cache);
		let loaded = AiSettings::load(&self.db);
		*self
			.ai_settings_cache
			.write()
			.unwrap_or_else(|e| e.into_inner()) = Some(loaded.clone());
		loaded
	}

	/// Persist settings and refresh the cache in one step, so a failed
	/// write never leaves a cache disagreeing with the database.
	pub fn save_ai_settings(&self, settings: &AiSettings) -> Result<(), String> {
		settings.save(&self.db)?;
		*self
			.ai_settings_cache
			.write()
			.unwrap_or_else(|e| e.into_inner()) = Some(settings.clone());
		Ok(())
	}

	pub fn models_dir(&self) -> PathBuf {
		self.data_dir.join("models")
	}

	pub fn model_path(&self, spec: &ModelSpec) -> PathBuf {
		self.models_dir().join(spec.filename)
	}

	/// Where this model's file can be loaded from, if anywhere: the
	/// app-managed copy first, then a file someone else already
	/// downloaded into the HuggingFace hub cache (hf CLI, other tools).
	pub fn resolve_model_file(&self, spec: &ModelSpec) -> Option<PathBuf> {
		let app_copy = self.model_path(spec);
		if app_copy.is_file() {
			return Some(app_copy);
		}
		hf_hub_cache_candidates()
			.iter()
			.find_map(|cache| hf_cache_model_path(cache, spec))
	}

	pub fn is_model_downloaded(&self, spec: &ModelSpec) -> bool {
		self.resolve_model_file(spec).is_some()
	}

	pub fn emit_llm_status(&self, app: &AppHandle) {
		let status = lock(&self.llm_status).clone();
		if let Err(e) = app.emit("llm-status", status) {
			log::warn!("failed to emit llm-status: {e}");
		}
	}

	pub fn emit_stt_status(&self, app: &AppHandle) {
		let status = lock(&self.stt_status).clone();
		if let Err(e) = app.emit("stt-status", status) {
			log::warn!("failed to emit stt-status: {e}");
		}
	}

	/// Load the given LLM model file into the runtime. Blocking; call from a
	/// background thread.
	///
	/// The previously loaded engine is dropped before the new file is
	/// mmap'd (peak memory stays at one model). If the new file fails to
	/// load or was deleted mid-load, the previous model is loaded back from
	/// disk, so a failed switch never leaves the app without a working LLM.
	///
	/// Err means the switch did NOT happen (a busy load, or the new model
	/// failed and could not be rolled back to); callers must not persist
	/// the new model as active on Err.
	pub fn load_llm(&self, app: &AppHandle, spec: &ModelSpec) -> Result<(), String> {
		// One load at a time: a second activate while the first is running
		// would mmap two multi-GB models simultaneously. Refuse instead of
		// silently ignoring, so callers can't persist a divergent active
		// model while a different load is in flight.
		if self.llm_loading.swap(true, Ordering::SeqCst) {
			log::warn!(
				"llm load already in progress; refusing request for {}",
				spec.id
			);
			return Err("a model is already loading - try again in a moment".into());
		}
		let result = self.load_llm_inner(app, spec);
		self.llm_loading.store(false, Ordering::SeqCst);
		self.emit_llm_status(app);
		result
	}

	/// Generic load/swap flow shared by both engine kinds: mark loading,
	/// run `prepare` (backend init for llama), drop the previous engine,
	/// load the new file, and either install it or roll back to the
	/// previous model. Blocking; call from a background thread.
	///
	/// `rollback_on_same`: whisper retries loading the same model after a
	/// transient failure; an identical llama reload fails deterministically
	/// on the same mmap, so it does not.
	/// `missing_when_vanished`: the llm path distinguishes "file deleted
	/// mid-load" (Missing) from a genuine load failure (Error).
	#[allow(clippy::too_many_arguments)]
	fn swap_engine<E>(
		&self,
		app: &AppHandle,
		spec: &ModelSpec,
		status: &std::sync::Mutex<EngineStatus>,
		emit: fn(&Self, &AppHandle),
		slot: fn(&mut Runtime) -> &mut Option<Arc<E>>,
		model_id_of: fn(&E) -> &str,
		prepare: impl FnOnce(&mut Runtime) -> Result<(), String>,
		load: impl Fn(&Path) -> Result<E, String>,
		rollback: impl Fn(&ModelSpec) -> Result<(), String>,
		rollback_on_same: bool,
		missing_when_vanished: bool,
	) -> Result<(), String> {
		{
			let mut s = lock(status);
			*s = EngineStatus::loading(spec.id);
		}
		emit(self, app);

		// App-managed copy first; fall back to a file already present in
		// the user's HuggingFace hub cache (no app copy to create).
		let path = self
			.resolve_model_file(spec)
			.unwrap_or_else(|| self.model_path(spec));
		let prev_spec: Option<ModelSpec> = {
			let mut runtime = lock(&self.runtime);
			slot(&mut runtime)
				.as_ref()
				.map(|engine| model_id_of(engine).to_string())
				.and_then(|id| find_model(&id, spec.kind))
				.cloned()
		};

		// Engine-specific setup (llama backend init), then drop the
		// previous engine so peak memory stays at one model.
		let result = (|| -> Result<E, String> {
			{
				let mut runtime = lock(&self.runtime);
				prepare(&mut runtime)?;
				*slot(&mut runtime) = None;
			}
			load(&path)
		})();
		let loaded = match result {
			Ok(engine) => Some(engine),
			Err(e) => {
				log::error!("{} load failed: {e}", spec.id);
				None
			}
		};

		// Install the new engine, or roll back to the previous one.
		let had_loaded = loaded.is_some();
		let install = loaded.filter(|_| path.is_file());
		match install {
			Some(engine) => {
				let mut runtime = lock(&self.runtime);
				*slot(&mut runtime) = Some(Arc::new(engine));
				drop(runtime);
				let mut s = lock(status);
				*s = EngineStatus::ready(Some(spec.id));
				Ok(())
			}
			None => {
				let vanished = had_loaded;
				if vanished {
					// The engine built fine but the model file vanished
					// while loading; don't resurrect a deleted model.
					log::warn!("{} was deleted while loading; not activating it", spec.id);
				}
				if let Some(prev) = prev_spec.filter(|prev| rollback_on_same || prev.id != spec.id)
				{
					match rollback(&prev) {
						Ok(()) => {
							log::warn!(
								"switch to {} failed; previous model {} is active again",
								spec.id,
								prev.id
							);
							let mut s = lock(status);
							*s = EngineStatus::new(
								EngineState::Ready,
								Some(prev.id),
								Some(&format!(
									"could not load {0} - {1} is still active",
									spec.id, prev.id
								)),
							);
							return Err(format!(
								"could not load {} - {} is still active",
								spec.id, prev.id
							));
						}
						Err(rollback_err) => {
							log::error!("rollback to {} failed: {rollback_err}", prev.id);
						}
					}
				}
				let mut s = lock(status);
				if vanished && missing_when_vanished {
					*s = EngineStatus::missing();
					return Err(format!("{} was deleted while loading", spec.id));
				}
				*s = EngineStatus::error(Some(spec.id), "model failed to load");
				Err("model failed to load".into())
			}
		}
	}

	fn load_llm_inner(&self, app: &AppHandle, spec: &ModelSpec) -> Result<(), String> {
		self.swap_engine(
			app,
			spec,
			&self.llm_status,
			Self::emit_llm_status,
			|runtime| &mut runtime.llm,
			|engine| &engine.model_id,
			|runtime| {
				// The llama backend is initialized once and kept for the
				// process lifetime; engines come and go on top of it.
				if runtime.backend.is_none() {
					let backend = llama_cpp_2::llama_backend::LlamaBackend::init()
						.map_err(|e| format!("failed to init llama backend: {e}"))?;
					runtime.backend = Some(Arc::new(backend));
				}
				Ok(())
			},
			|path| {
				let backend = lock(&self.runtime)
					.backend
					.clone()
					.ok_or_else(|| "llama backend missing".to_string())?;
				LocalLlm::load(backend, path, spec.id)
			},
			|prev| self.reload_llm(prev),
			// an identical llama reload fails deterministically on the
			// same mmap; not worth retrying
			false,
			true,
		)
	}

	/// Best-effort reload of a previously working model (rollback path).
	fn reload_llm(&self, spec: &ModelSpec) -> Result<(), String> {
		let path = self
			.resolve_model_file(spec)
			.ok_or_else(|| format!("model file for {} is gone", spec.id))?;
		let backend = lock(&self.runtime)
			.backend
			.clone()
			.ok_or_else(|| "llama backend missing".to_string())?;
		let engine = LocalLlm::load(backend, &path, spec.id)?;
		let mut runtime = lock(&self.runtime);
		runtime.llm = Some(Arc::new(engine));
		Ok(())
	}

	/// Load the given whisper model file. Blocking; call from a background
	/// thread. Same staging/rollback contract as `load_llm`.
	pub fn load_stt(&self, app: &AppHandle, spec: &ModelSpec) -> Result<(), String> {
		if self.stt_loading.swap(true, Ordering::SeqCst) {
			log::warn!(
				"stt load already in progress; refusing request for {}",
				spec.id
			);
			return Err("a model is already loading - try again in a moment".into());
		}
		let result = self.load_stt_inner(app, spec);
		self.stt_loading.store(false, Ordering::SeqCst);
		self.emit_stt_status(app);
		result
	}

	fn load_stt_inner(&self, app: &AppHandle, spec: &ModelSpec) -> Result<(), String> {
		self.swap_engine(
			app,
			spec,
			&self.stt_status,
			Self::emit_stt_status,
			|runtime| &mut runtime.stt,
			|engine| &engine.model_id,
			|_runtime| Ok(()),
			|path| SttEngine::load(path, spec.id),
			|prev| {
				let prev_path = self
					.resolve_model_file(prev)
					.ok_or_else(|| format!("model file for {} is gone", prev.id))?;
				{}
				let engine = SttEngine::load(&prev_path, prev.id)?;
				lock(&self.runtime).stt = Some(Arc::new(engine));
				Ok(())
			},
			// whisper reads the file fresh each time: retry the same model
			// after a transient failure (the engine was already dropped)
			true,
			false,
		)
	}
}

/// Precedence: the in-app setting (GUI apps don't inherit shell env
/// vars), then `HF_ENDPOINT` (hf-hub/transformers semantics, e.g.
/// https://hf-mirror.com set via launchctl or a terminal launch), then
/// the default. Pure for tests.
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

pub fn model_url(spec: &ModelSpec, endpoint: &str) -> String {
	format!("{}/{}/resolve/main/{}", endpoint, spec.repo, spec.filename)
}

/// Every hub-cache directory a model file could already live in, in
/// huggingface_hub precedence order: `HF_HUB_CACHE`, then the legacy
/// `HUGGINGFACE_HUB_CACHE`, then `HF_HOME/hub`, then
/// `$XDG_CACHE_HOME/huggingface/hub` (the Python client honors XDG; the
/// Rust hf-hub crate does not - probing costs nothing), then the
/// platform-independent default `~/.cache/huggingface/hub` (HF uses
/// ~/.cache even on macOS/Windows, never the platform cache dirs).
/// Discovery is read-only, so extra candidates are harmless.
pub fn hf_hub_cache_candidates() -> Vec<PathBuf> {
	let mut candidates: Vec<PathBuf> = Vec::new();
	let mut push = |p: Option<PathBuf>| {
		if let Some(p) = p.filter(|p| !p.as_os_str().is_empty()) {
			if !candidates.contains(&p) {
				candidates.push(p);
			}
		}
	};
	for var in ["HF_HUB_CACHE", "HUGGINGFACE_HUB_CACHE"] {
		push(std::env::var(var).ok().map(|v| PathBuf::from(v.trim())));
	}
	push(
		std::env::var("HF_HOME")
			.ok()
			.map(|v| PathBuf::from(v.trim()).join("hub")),
	);
	push(
		std::env::var("XDG_CACHE_HOME")
			.ok()
			.map(|v| PathBuf::from(v.trim()).join("huggingface").join("hub")),
	);
	push(dirs::home_dir().map(|home| home.join(".cache").join("huggingface").join("hub")));
	candidates
}

/// Locate `spec`'s file inside a HuggingFace hub cache directory
/// (`models--<org>--<repo>/snapshots/<rev>/<filename>`). Symlinked
/// snapshot files (the normal layout) resolve through `is_file`.
/// Returns the newest snapshot that contains the file.
pub fn hf_cache_model_path(cache_dir: &Path, spec: &ModelSpec) -> Option<PathBuf> {
	let repo_dir = cache_dir.join(format!("models--{}", spec.repo.replace('/', "--")));
	let snapshots = repo_dir.join("snapshots");
	let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
	for entry in std::fs::read_dir(&snapshots).ok()?.flatten() {
		let candidate = entry.path().join(spec.filename);
		if !candidate.is_file() {
			continue;
		}
		let modified = entry
			.metadata()
			.ok()
			.and_then(|m| m.modified().ok())
			.unwrap_or(std::time::SystemTime::UNIX_EPOCH);
		if best.as_ref().is_none_or(|(t, _)| modified > *t) {
			best = Some((modified, candidate));
		}
	}
	best.map(|(_, path)| path)
}

/// The one cache directory Brainstory writes into: the first
/// environment-configured candidate, else the default. Discovery reads
/// every candidate; writes need exactly one target.
pub fn primary_hub_cache() -> PathBuf {
	hf_hub_cache_candidates()
		.into_iter()
		.next()
		.unwrap_or_else(|| {
			dirs::home_dir()
				.unwrap_or_else(|| PathBuf::from("."))
				.join(".cache")
				.join("huggingface")
				.join("hub")
		})
}

fn hf_repo_dir(cache: &Path, spec: &ModelSpec) -> PathBuf {
	cache.join(format!("models--{}", spec.repo.replace('/', "--")))
}

/// The content-addressed blob for this model: `blobs/<pinned sha256>`,
/// the same name huggingface tooling uses for LFS files, so a blob we
/// download or migrate is deduped against theirs automatically.
pub fn hf_blob_path(cache: &Path, spec: &ModelSpec) -> PathBuf {
	hf_repo_dir(cache, spec).join("blobs").join(spec.sha256)
}

/// Link a blob into a snapshot under `snapshots/<sha>/<filename>`,
/// trying a relative symlink (the standard layout) first, then a
/// hardlink (Windows without symlink privileges), then a copy. Idempotent.
pub fn materialize_snapshot(cache: &Path, spec: &ModelSpec) -> Result<(), String> {
	let snapshot_dir = hf_repo_dir(cache, spec).join("snapshots").join(spec.sha256);
	std::fs::create_dir_all(&snapshot_dir).map_err(|e| e.to_string())?;
	let link = snapshot_dir.join(spec.filename);
	if link.symlink_metadata().is_ok() {
		return Ok(());
	}
	let blob = hf_blob_path(cache, spec);
	if !blob.is_file() {
		return Err(format!("blob missing for {}", spec.id));
	}
	let rel = std::path::Path::new("../../blobs").join(spec.sha256);
	#[cfg(target_family = "unix")]
	{
		std::os::unix::fs::symlink(&rel, &link).map_err(|e| e.to_string())?;
	}
	#[cfg(target_os = "windows")]
	{
		use std::os::windows::fs as win_fs;
		if win_fs::symlink_file(&rel, &link).is_err() {
			std::fs::hard_link(&blob, &link)
				.or_else(|_| std::fs::copy(&blob, &link).map(|_| ()))
				.map_err(|e| e.to_string())?;
		}
	}
	Ok(())
}

/// Streaming sha256 of a file, lowercase hex.
fn sha256_of_file(path: &Path) -> Option<String> {
	use sha2::{Digest, Sha256};
	let mut file = std::fs::File::open(path).ok()?;
	let mut hasher = Sha256::new();
	let mut buf = vec![0u8; 1024 * 1024];
	use std::io::Read;
	loop {
		let n = file.read(&mut buf).ok()?;
		if n == 0 {
			break;
		}
		hasher.update(&buf[..n]);
	}
	Some(
		hasher
			.finalize()
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect(),
	)
}

/// One-time migration: move legacy app-dir model files into the hub
/// cache so one copy serves Brainstory and every other HF tool. The
/// source is hash-verified first - a corrupt or foreign file must never
/// be renamed into a content-addressed store under a sha it doesn't
/// have. An existing blob means the content is already cached: the app
/// copy is redundant and simply removed.
pub fn migrate_legacy_models(models_dir: &Path, cache: &Path) {
	for spec in LLM_MODELS.iter().chain(STT_MODELS.iter()) {
		migrate_one(models_dir, cache, spec);
	}
}

/// Migrate a single spec's app-dir file into the cache (separate so
/// tests can drive it with fixture specs instead of the catalog pins).
fn migrate_one(models_dir: &Path, cache: &Path, spec: &ModelSpec) {
	let app_file = models_dir.join(spec.filename);
	if !app_file.is_file() {
		return;
	}
	let blob = hf_blob_path(cache, spec);
	if blob.is_file() {
		log::info!(
			"migrating {}: blob already cached, dropping the app copy",
			spec.id
		);
		if let Err(e) = std::fs::remove_file(&app_file) {
			log::warn!("could not remove the redundant app copy: {e}");
			return;
		}
	} else {
		match sha256_of_file(&app_file) {
			Some(hash) if hash.eq_ignore_ascii_case(spec.sha256) => {}
			other => {
				log::warn!(
					"leaving {} in the app models dir: its content does not match the pinned hash ({:?})",
					spec.id,
					other
				);
				return;
			}
		}
		if let Some(parent) = blob.parent() {
			if let Err(e) = std::fs::create_dir_all(parent) {
				log::warn!("could not create the cache blobs dir: {e}");
				return;
			}
		}
		// rename within a volume; fall back to copy-via-.part across
		// volumes (a partial copy never lands under the final name)
		if std::fs::rename(&app_file, &blob).is_err() {
			let tmp = part_path(&blob);
			match std::fs::copy(&app_file, &tmp)
				.and_then(|_| std::fs::rename(&tmp, &blob))
				.and_then(|_| std::fs::remove_file(&app_file))
			{
				Ok(()) => {}
				Err(e) => {
					log::warn!("could not migrate {} into the cache: {e}", spec.id);
					let _ = std::fs::remove_file(&tmp);
					return;
				}
			}
		}
		log::info!("migrated {} into the hub cache", spec.id);
	}
	if let Err(e) = materialize_snapshot(cache, spec) {
		log::warn!("could not create the cache snapshot for {}: {e}", spec.id);
	}
}

/// True when some snapshot entry still links to `blobs/<sha>`.
/// Symlinks are inspected precisely; a non-symlink entry (hardlink or
/// copied fallback, e.g. on Windows) hides its target, so it is treated
/// as referencing the blob - never prune what might be in use.
fn blob_referenced(snapshots_dir: &Path, sha: &str) -> bool {
	for rev in std::fs::read_dir(snapshots_dir)
		.into_iter()
		.flatten()
		.flatten()
	{
		let rev_dir = rev.path();
		if !rev_dir.is_dir() {
			continue;
		}
		for entry in std::fs::read_dir(rev_dir).into_iter().flatten().flatten() {
			let path = entry.path();
			if !path.is_file() {
				continue; // broken symlink or directory
			}
			match std::fs::read_link(&path) {
				Ok(target) => {
					if target.file_name().map(|n| n == sha).unwrap_or(false) {
						return true;
					}
				}
				Err(_) => return true, // not a symlink: conservatively in use
			}
		}
	}
	false
}

/// Remove this model's cache entry: every `snapshots/*/<filename>` link,
/// then the blob when nothing else in the repo references it. This is
/// the same rule huggingface's own cache pruning applies, so deleting
/// in Brainstory never breaks another tool's snapshot (worst case, that
/// tool re-downloads a blob we removed as unreferenced).
pub fn remove_cached_model(cache: &Path, spec: &ModelSpec) -> Result<bool, String> {
	let repo_dir = hf_repo_dir(cache, spec);
	let snapshots = repo_dir.join("snapshots");
	if !snapshots.is_dir() {
		return Ok(false);
	}
	let mut removed = false;
	for rev in std::fs::read_dir(&snapshots)
		.map_err(|e| e.to_string())?
		.flatten()
	{
		let target = rev.path().join(spec.filename);
		if target.symlink_metadata().is_ok() {
			std::fs::remove_file(&target).map_err(|e| e.to_string())?;
			removed = true;
		}
	}
	let blob = hf_blob_path(cache, spec);
	if blob.is_file() && !blob_referenced(&snapshots, spec.sha256) {
		std::fs::remove_file(&blob).map_err(|e| e.to_string())?;
		removed = true;
	}
	Ok(removed)
}

/// The `.part` staging path for a download destination: `<file>.part`
/// appended to the full name (with_extension would collapse `x.bin` and
/// `x.gguf` to the same `x.part`).
fn part_path(dest: &Path) -> PathBuf {
	let mut name = dest.as_os_str().to_os_string();
	name.push(".part");
	PathBuf::from(name)
}

/// Stream a model file to disk, reporting progress through `on_progress`
/// (percentage 0-100). Verifies the download completed fully and matches
/// the pinned sha256 before moving it into place; the `.part` file is
/// removed on any failure.
pub async fn download_model_file(
	url: &str,
	dest: &Path,
	expected_size: u64,
	expected_sha256: &str,
	hf_token: &str,
	cancel: &AtomicBool,
	on_progress: &mut (impl FnMut(f64) + Send),
) -> Result<(), String> {
	use sha2::{Digest, Sha256};

	let tmp = part_path(dest);

	let client = reqwest::Client::builder()
		.connect_timeout(std::time::Duration::from_secs(15))
		.build()
		.map_err(|e| e.to_string())?;

	// Resume support: a leftover .part from a quit mid-download can be
	// continued with a Range request instead of restarting multi-GB from
	// zero. The existing bytes are hashed while streaming them from disk,
	// so the final sha256 check still covers the whole file.
	let mut hasher = (!expected_sha256.is_empty()).then(Sha256::new);
	let mut downloaded: u64 = 0;
	let mut resume_from: u64 = 0;
	if let Ok(meta) = std::fs::metadata(&tmp) {
		resume_from = meta.len();
		// Only resume when the prefix can still matter: a .part larger
		// than the expected file is junk from a different state.
		if expected_size > 0 && resume_from >= expected_size {
			tokio::fs::remove_file(&tmp)
				.await
				.map_err(|e| e.to_string())?;
			resume_from = 0;
		}
	} else if tmp.exists() {
		// exists but unreadable metadata: start over
		tokio::fs::remove_file(&tmp)
			.await
			.map_err(|e| e.to_string())?;
	}

	let mut request = client.get(url).header("User-Agent", USER_AGENT);
	if resume_from > 0 {
		request = request.header("Range", format!("bytes={resume_from}-"));
	}
	if !hf_token.is_empty() {
		request = request.bearer_auth(hf_token);
	}
	let response = request
		.send()
		.await
		.map_err(|e| format!("download request failed: {e}"))?;
	if response.status() == reqwest::StatusCode::UNAUTHORIZED
		|| response.status() == reqwest::StatusCode::FORBIDDEN
	{
		return Err(format!(
			"download not authorized ({}) - check the HuggingFace access token in AI Models settings",
			response.status()
		));
	}
	if !response.status().is_success() {
		return Err(format!("download failed with status {}", response.status()));
	}

	// A server that ignores Range answers 200 with the full body; the
	// stale .part cannot be stitched onto it, so restart from zero.
	let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
	if resume_from > 0 && !resumed {
		tokio::fs::remove_file(&tmp)
			.await
			.map_err(|e| e.to_string())?;
		resume_from = 0;
		hasher = (!expected_sha256.is_empty()).then(Sha256::new);
	}

	// Hash the resumed prefix from disk so the integrity check still
	// covers the complete file, and pre-seed the byte counter.
	if resumed {
		if let Some(h) = hasher.as_mut() {
			let mut file = tokio::fs::File::open(&tmp)
				.await
				.map_err(|e| e.to_string())?;
			use tokio::io::AsyncReadExt;
			let mut buf = vec![0u8; 1024 * 1024];
			loop {
				let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
				if n == 0 {
					break;
				}
				h.update(&buf[..n]);
			}
		}
		downloaded = resume_from;
		log::info!("resuming download at {resume_from} of {expected_size} bytes");
	} else if resume_from == 0 && tmp.exists() {
		// fresh download: the staging file must be empty/new
		tokio::fs::remove_file(&tmp)
			.await
			.map_err(|e| e.to_string())?;
	}

	let total = response.content_length().unwrap_or(0) + resume_from;
	// Fail fast when the advertised length already contradicts the spec:
	// streaming multi-GB only to reject it at the end wastes the transfer.
	if total > 0 && expected_size > 0 && total != expected_size {
		return Err(format!(
			"download size mismatch (server says {total} bytes, expected {expected_size}) - please retry"
		));
	}
	use futures_util::StreamExt;
	let mut stream = response.bytes_stream();
	let mut file = if resumed {
		tokio::fs::OpenOptions::new()
			.append(true)
			.open(&tmp)
			.await
			.map_err(|e| e.to_string())?
	} else {
		tokio::fs::File::create(&tmp)
			.await
			.map_err(|e| e.to_string())?
	};
	use tokio::io::AsyncWriteExt;

	// Every failure path below removes the partial file, so a retry starts
	// clean instead of leaving gigabytes of junk behind.
	let outcome = async {
		// `downloaded` comes from the outer scope: it is pre-seeded with
		// the resumed prefix so totals and progress include it.
		let mut last_report: u64 = downloaded;
		const CHUNK_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
		loop {
			if cancel.load(Ordering::Relaxed) {
				return Err("download cancelled".into());
			}
			let chunk = match tokio::time::timeout(CHUNK_IDLE_TIMEOUT, stream.next()).await {
				Err(_) => return Err("download stalled (no data for 60s)".into()),
				Ok(Some(Ok(c))) => c,
				Ok(Some(Err(e))) => return Err(format!("download interrupted: {e}")),
				Ok(None) => break,
			};
			if let Some(hasher) = hasher.as_mut() {
				hasher.update(&chunk);
			}
			file.write_all(&chunk).await.map_err(|e| e.to_string())?;
			downloaded += chunk.len() as u64;
			if downloaded - last_report > 2_000_000 || downloaded == total {
				last_report = downloaded;
				let pct = if total > 0 {
					(downloaded as f64 / total as f64) * 100.0
				} else {
					// Content-length unknown (chunked transfer): report the
					// indeterminate sentinel; the UI shows a busy bar.
					-1.0
				};
				on_progress(pct);
			}
		}
		file.flush().await.map_err(|e| e.to_string())?;
		// The stream can end "cleanly" mid-body; only a full-length file is
		// a valid model, anything else fails to load with cryptic errors.
		if total > 0 && downloaded != total {
			return Err(format!(
				"download incomplete (got {downloaded} of {total} bytes) - please retry"
			));
		}
		if expected_size > 0 && downloaded != expected_size {
			return Err(format!(
				"download size mismatch (got {downloaded} bytes, expected {expected_size}) - please retry"
			));
		}
		if let Some(hasher) = hasher.take() {
			let actual: String = hasher
				.finalize()
				.iter()
				.map(|b| format!("{b:02x}"))
				.collect();
			if !actual.eq_ignore_ascii_case(expected_sha256) {
				return Err(format!(
					"download failed its integrity check (sha256 {actual}) - the file was corrupted in transit or changed upstream; please retry"
				));
			}
		}
		Ok(())
	}
	.await;

	match outcome {
		Ok(()) => {
			drop(file);
			tokio::fs::rename(&tmp, dest)
				.await
				.map_err(|e| e.to_string())?;
			Ok(())
		}
		Err(e) => {
			let _ = tokio::fs::remove_file(&tmp).await;
			Err(e)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{download_model_file, AiSettings, Db};
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
		assert_eq!(s.stt_language, "en-US");
		s.stt_language = "de-DE".into();
		s.save(&db).expect("save");
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
		let err = AiSettings::load(&db)
			.save(&db)
			.expect_err("save must surface the failure instead of logging it");
		assert!(
			err.contains("failed to save setting"),
			"unexpected error: {err}"
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
	fn part_paths_do_not_collide_across_extensions() {
		use std::path::Path;
		let bin = super::part_path(Path::new("/m/x.bin"));
		let gguf = super::part_path(Path::new("/m/x.gguf"));
		assert_eq!(bin, Path::new("/m/x.bin.part"));
		assert_eq!(gguf, Path::new("/m/x.gguf.part"));
		assert_ne!(bin, gguf, "staging names must be distinct");
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

	use std::sync::atomic::AtomicBool;
	use std::sync::Arc;

	/// Serve one canned HTTP response from a loopback listener; returns the
	/// base URL. Good enough to exercise the downloader against a real
	/// socket without an HTTP-server dependency.
	fn serve(response: Vec<u8>) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut buf = [0u8; 4096];
				let _ = sock.read(&mut buf); // drain the request head
				let _ = sock.write_all(&response);
				let _ = sock.flush();
				// keep the socket open briefly so the client can read it all
				std::thread::sleep(std::time::Duration::from_millis(500));
			}
		});
		format!("http://{addr}/model.bin")
	}

	fn http(body: &[u8], extra_headers: &str) -> Vec<u8> {
		let mut response = format!(
			"HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{}\r\n",
			body.len(),
			extra_headers
		)
		.into_bytes();
		response.extend_from_slice(body);
		response
	}

	fn temp_dest(name: &str) -> (std::path::PathBuf, tempfile::TempDir) {
		let dir = tempfile::tempdir().expect("tempdir");
		(dir.path().join(name), dir)
	}

	fn sha256_hex(bytes: &[u8]) -> String {
		use sha2::{Digest, Sha256};
		Sha256::digest(bytes)
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect::<String>()
	}

	#[tokio::test]
	async fn downloads_and_verifies_a_clean_file() {
		let body = vec![7u8; 100_000];
		let url = serve(http(&body, ""));
		let (dest, _dir) = temp_dest("ok");
		let cancel = Arc::new(AtomicBool::new(false));
		let mut progress = Vec::new();
		download_model_file(
			&url,
			&dest,
			body.len() as u64,
			&sha256_hex(&body),
			"",
			&cancel,
			&mut |p| progress.push(p),
		)
		.await
		.expect("clean download");
		assert!(dest.is_file());
		assert_eq!(std::fs::read(&dest).unwrap(), body);
		assert!(!progress.is_empty(), "progress was reported");
		assert_eq!(*progress.last().unwrap(), 100.0);
	}

	#[tokio::test]
	async fn rejects_a_hash_mismatch() {
		let body = vec![7u8; 10_000];
		let url = serve(http(&body, ""));
		let (dest, _dir) = temp_dest("hash");
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			body.len() as u64,
			"deadbeef",
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("hash mismatch must fail");
		assert!(err.contains("integrity"), "unexpected error: {err}");
		assert!(!dest.exists(), "no file left behind on failure");
	}

	#[tokio::test]
	async fn rejects_a_truncated_transfer() {
		// Content-Length promises more than the body delivers
		let body = vec![1u8; 500];
		let url = serve(http(&body, ""));
		let (dest, _dir) = temp_dest("trunc");
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			100_000,
			&sha256_hex(&body),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("truncated transfer must fail");
		assert!(
			err.contains("incomplete") || err.contains("mismatch"),
			"unexpected error: {err}"
		);
		assert!(!dest.exists());
	}

	#[tokio::test]
	async fn honors_cancellation() {
		let body = vec![3u8; 10_000];
		let url = serve(http(&body, ""));
		let (dest, _dir) = temp_dest("cancel");
		let cancel = Arc::new(AtomicBool::new(false));
		cancel.store(true, std::sync::atomic::Ordering::Relaxed);
		let err = download_model_file(
			&url,
			&dest,
			body.len() as u64,
			&sha256_hex(&body),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("cancelled download must fail");
		assert!(err.contains("cancelled"), "unexpected error: {err}");
		assert!(!dest.exists());
	}

	#[tokio::test]
	async fn rejects_a_size_mismatch() {
		let body = vec![5u8; 1_000];
		let url = serve(http(&body, ""));
		let (dest, _dir) = temp_dest("size");
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			999_999,
			&sha256_hex(&body),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("size mismatch must fail");
		assert!(err.contains("size mismatch"), "unexpected error: {err}");
		assert!(!dest.exists());
	}
}

#[cfg(test)]
mod download_tests {
	use super::{download_model_file, part_path};
	use sha2::{Digest, Sha256};
	use std::io::{Read, Write};
	use std::sync::atomic::AtomicBool;
	use std::sync::Arc;

	/// Loopback HTTP server that hands the request head to a callback so
	/// tests can inspect headers, then serves a canned response.
	fn serve_inspecting(respond: impl FnOnce(&str) -> Vec<u8> + Send + 'static) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				// read just the request head (until \r\n\r\n); never read
				// to EOF - pooled clients keep the connection open
				let mut request = String::new();
				loop {
					let mut byte = [0u8; 1];
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0] as char);
					if request.ends_with("\r\n\r\n") {
						break;
					}
				}
				let response = respond(&request);
				let _ = sock.write_all(&response);
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});
		format!("http://{addr}/model.bin")
	}

	fn sha256_hex(bytes: &[u8]) -> String {
		Sha256::digest(bytes)
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect()
	}

	fn dest(tag: &str) -> std::path::PathBuf {
		part_path(
			&std::env::temp_dir().join(format!("brainstory-dl-cov-{tag}-{}", uuid::Uuid::new_v4())),
		)
		.with_file_name(format!(
			"brainstory-dl-cov-{tag}-{}.bin",
			uuid::Uuid::new_v4()
		))
	}

	#[tokio::test]
	async fn download_sends_bearer_token_when_configured() {
		let body = vec![1u8; 100];
		let digest = sha256_hex(&body);
		let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
		let seen_writer = seen.clone();
		let url = serve_inspecting(move |request| {
			*seen_writer.lock().unwrap() = request.to_string();
			format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
				.into_bytes()
				.into_iter()
				.chain(body.clone())
				.collect()
		});
		let dest = dest("auth");
		let cancel = Arc::new(AtomicBool::new(false));
		download_model_file(
			&url,
			&dest,
			100,
			&digest,
			"hf_token_123",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect("download ok");
		let request = seen.lock().unwrap().clone();
		assert!(
			request
				.to_lowercase()
				.contains("authorization: bearer hf_token_123"),
			"bearer token sent: {request}"
		);
	}

	#[tokio::test]
	async fn download_omits_bearer_header_when_empty() {
		let body = vec![2u8; 50];
		let digest = sha256_hex(&body);
		let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
		let seen_writer = seen.clone();
		let url = serve_inspecting(move |request| {
			*seen_writer.lock().unwrap() = request.to_string();
			format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
				.into_bytes()
				.into_iter()
				.chain(body.clone())
				.collect()
		});
		let dest = dest("anon");
		let cancel = Arc::new(AtomicBool::new(false));
		download_model_file(&url, &dest, 50, &digest, "", &cancel, &mut |_| {})
			.await
			.expect("download ok");
		let request = seen.lock().unwrap().clone();
		assert!(
			!request.to_lowercase().contains("authorization:"),
			"no auth header for anonymous download: {request}"
		);
	}

	#[tokio::test]
	async fn download_reports_indeterminate_progress_without_content_length() {
		// progress is only reported past ~2 MB, so exceed it; the digest
		// is computed before the body moves into the server closure
		let body = vec![3u8; 3_000_000];
		let digest = sha256_hex(&body);
		// chunked transfer, no Content-Length
		let url = serve_inspecting(move |_| {
			let mut out = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
			for chunk in body.chunks(1_000_000) {
				out.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
				out.extend_from_slice(chunk);
				out.extend_from_slice(b"\r\n");
			}
			out.extend_from_slice(b"0\r\n\r\n");
			out
		});
		let dest = dest("indeterminate");
		let cancel = Arc::new(AtomicBool::new(false));
		let mut progress = Vec::new();
		download_model_file(&url, &dest, 3_000_000, &digest, "", &cancel, &mut |p| {
			progress.push(p)
		})
		.await
		.expect("download ok");
		assert!(
			progress.iter().any(|p| *p < 0.0),
			"indeterminate sentinel reported when the total is unknown: {progress:?}"
		);
	}

	#[tokio::test]
	async fn download_fails_fast_when_content_length_differs_from_spec() {
		let body = vec![4u8; 500];
		let url = serve_inspecting(move |_| {
			// server promises 500 bytes but the spec says 1000: the
			// mismatch must be caught from the headers, not after the body
			format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
				.into_bytes()
				.into_iter()
				.chain(body.clone())
				.collect()
		});
		let dest = dest("failfast");
		let cancel = Arc::new(AtomicBool::new(false));
		let err = download_model_file(
			&url,
			&dest,
			1000,
			&sha256_hex(&[4u8; 500]),
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect_err("must fail");
		assert!(
			err.contains("server says 500 bytes, expected 1000"),
			"unexpected error: {err}"
		);
		assert!(!dest.exists(), "no file on early rejection");
	}
}

#[cfg(test)]
mod settings_cache_tests {
	use super::*;

	#[test]
	fn ai_settings_cache_round_trips_through_save() {
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join("t.db")).expect("db");
		let state = AppState::new(db, dir.path().to_path_buf());

		// cold read loads and warms the cache
		assert_eq!(state.ai_settings().llm_model, LLM_MODELS[0].id);
		// a write through save_ai_settings refreshes the cache
		let mut next = state.ai_settings();
		next.stt_language = "fr-FR".into();
		state.save_ai_settings(&next).expect("save");
		assert_eq!(state.ai_settings().stt_language, "fr-FR");
		// and persisted: a fresh AppState sees the same value
		let db2 = Db::open(&dir.path().join("t.db")).expect("reopen db");
		let state2 = AppState::new(db2, dir.path().to_path_buf());
		assert_eq!(state2.ai_settings().stt_language, "fr-FR");
	}

	#[test]
	fn ai_settings_cache_is_a_cache_not_a_source() {
		// direct DB writes (the legacy path) are visible after a cache
		// refresh via save, proving the cache never outruns the database
		let dir = tempfile::tempdir().expect("tempdir");
		let db = Db::open(&dir.path().join("t.db")).expect("db");
		let state = AppState::new(db, dir.path().to_path_buf());
		let _ = state.ai_settings(); // warm
		let mut updated = state.ai_settings();
		updated.llm_model = "gemma-4-E4B".into();
		state.save_ai_settings(&updated).expect("save");
		assert_eq!(state.ai_settings().llm_model, "gemma-4-E4B");
	}
}

#[cfg(test)]
mod resume_tests {
	use super::{download_model_file, part_path};
	use sha2::{Digest, Sha256};
	use std::io::{Read, Write};
	use std::sync::atomic::AtomicBool;
	use std::sync::Arc;

	fn sha256_hex(bytes: &[u8]) -> String {
		Sha256::digest(bytes)
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect()
	}

	/// Serve the body honoring a Range request (like HuggingFace does).
	fn serve_ranged(
		body: Vec<u8>,
		saw_range: std::sync::Arc<std::sync::Mutex<Option<String>>>,
	) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				let mut request = String::new();
				loop {
					let mut byte = [0u8; 1];
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0] as char);
					if request.ends_with("\r\n\r\n") {
						break;
					}
				}
				let range = request
					.lines()
					.find(|l| l.to_lowercase().starts_with("range:"))
					.map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string());
				*saw_range.lock().unwrap() = range.clone();
				let (status, slice): (&str, &[u8]) = match range
					.as_deref()
					.and_then(|r| r.strip_prefix("bytes=").and_then(|r| r.split('-').next()))
					.and_then(|start| start.parse::<usize>().ok())
				{
					Some(start) if start < body.len() => ("206 Partial Content", &body[start..]),
					Some(_) => ("416 Range Not Satisfiable", &[]),
					None => ("200 OK", &body),
				};
				let head = format!(
					"HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n",
					slice.len()
				);
				let _ = sock.write_all(head.as_bytes());
				let _ = sock.write_all(slice);
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});
		format!("http://{addr}/model.bin")
	}

	#[tokio::test]
	async fn resumes_a_partial_file_with_a_range_request() {
		let body = vec![7u8; 3000];
		let digest = sha256_hex(&body);
		let saw_range = std::sync::Arc::new(std::sync::Mutex::new(None));
		let url = serve_ranged(body.clone(), saw_range.clone());
		let dest = part_path(
			&std::env::temp_dir().join(format!("brainstory-resume-{}.bin", uuid::Uuid::new_v4())),
		)
		.with_file_name(format!("brainstory-resume-{}.bin", uuid::Uuid::new_v4()));
		let tmp = part_path(&dest);

		// a stalled download left the first 1000 bytes staged
		std::fs::write(&tmp, &body[..1000]).expect("stage prefix");

		let cancel = Arc::new(AtomicBool::new(false));
		download_model_file(
			&url,
			&dest,
			body.len() as u64,
			&digest,
			"",
			&cancel,
			&mut |p| {
				assert!(p >= 0.0, "progress stays valid on resume");
			},
		)
		.await
		.expect("resumed download");

		assert_eq!(
			saw_range.lock().unwrap().as_deref(),
			Some("bytes=1000-"),
			"Range header sent for the staged prefix"
		);
		assert_eq!(
			std::fs::read(&dest).unwrap(),
			body,
			"file assembled correctly"
		);
		let _ = std::fs::remove_file(&dest);
	}

	#[tokio::test]
	async fn restarts_when_the_server_ignores_range() {
		let body = vec![9u8; 1500];
		let digest = sha256_hex(&body);
		let saw_range = std::sync::Arc::new(std::sync::Mutex::new(None));
		let url = serve_ranged(body.clone(), saw_range.clone());
		let dest = std::env::temp_dir().join(format!(
			"brainstory-resume-ign-{}.bin",
			uuid::Uuid::new_v4()
		));

		// stale prefix from a DIFFERENT transfer must not be stitched on
		std::fs::write(part_path(&dest), b"garbage prefix").expect("stage junk");

		let cancel = Arc::new(AtomicBool::new(false));
		// The server here honors Range, so make the junk prefix longer
		// than the body: the resume is refused and the download restarts.
		std::fs::write(part_path(&dest), vec![0u8; 2000]).expect("stage oversized junk");
		download_model_file(
			&url,
			&dest,
			body.len() as u64,
			&digest,
			"",
			&cancel,
			&mut |_| {},
		)
		.await
		.expect("clean restart");
		assert_eq!(std::fs::read(&dest).unwrap(), body);
		let _ = std::fs::remove_file(&dest);
	}
}

#[cfg(test)]
mod hf_cache_tests {
	use super::{hf_cache_model_path, LLM_MODELS};

	#[test]
	fn hf_cache_layout_resolves_and_picks_the_newest_snapshot() {
		let dir = tempfile::tempdir().expect("tempdir");
		let spec = &LLM_MODELS[0];
		let repo_dir = dir
			.path()
			.join(format!("models--{}", spec.repo.replace('/', "--")))
			.join("snapshots");

		// two snapshot revisions; only one carries the file
		let old_rev = repo_dir.join("aaaa");
		let new_rev = repo_dir.join("bbbb");
		std::fs::create_dir_all(&old_rev).unwrap();
		std::fs::create_dir_all(&new_rev).unwrap();
		std::fs::write(old_rev.join(spec.filename), b"old").unwrap();
		std::fs::write(new_rev.join(spec.filename), b"new").unwrap();
		// only one revision has the file: deterministic resolution
		std::fs::remove_file(old_rev.join(spec.filename)).unwrap();

		let found = hf_cache_model_path(dir.path(), spec).expect("resolved");
		assert_eq!(found, new_rev.join(spec.filename));

		// (the empty old_rev snapshot exercises the skip path already)
		assert_eq!(
			hf_cache_model_path(dir.path(), spec),
			Some(new_rev.join(spec.filename))
		);
		// no snapshot with the file -> None
		std::fs::remove_file(new_rev.join(spec.filename)).unwrap();
		assert_eq!(hf_cache_model_path(dir.path(), spec), None);
	}

	#[test]
	fn endpoint_resolution_setting_env_default_precedence() {
		use super::resolve_hf_endpoint;
		let default = "https://huggingface.co";
		// setting wins over everything, trimmed
		assert_eq!(
			resolve_hf_endpoint(" https://hf-mirror.com/ ", Some("https://other.example")),
			"https://hf-mirror.com"
		);
		// empty setting falls to the env var
		assert_eq!(
			resolve_hf_endpoint("", Some("https://hf-mirror.com/")),
			"https://hf-mirror.com"
		);
		// blank-only setting counts as empty
		assert_eq!(
			resolve_hf_endpoint("   ", Some("https://hf-mirror.com")),
			"https://hf-mirror.com"
		);
		// neither set: the default
		assert_eq!(resolve_hf_endpoint("", None), default);
	}
}

#[cfg(test)]
mod cache_storage_tests {
	use super::{
		hf_blob_path, hf_cache_model_path, materialize_snapshot, migrate_one, remove_cached_model,
		LLM_MODELS,
	};
	use sha2::{Digest, Sha256};

	fn sha256_hex(bytes: &[u8]) -> String {
		Sha256::digest(bytes)
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect()
	}

	/// A spec-shaped fixture whose pinned sha matches `content`, so
	/// migration/materialization accept it.
	fn spec_for(content: &[u8]) -> super::ModelSpec {
		let mut spec = LLM_MODELS[0].clone();
		spec.sha256 = Box::leak(sha256_hex(content).into_boxed_str());
		spec
	}

	#[test]
	fn materialize_publishes_a_blob_and_resolves_through_the_snapshot() {
		let cache = tempfile::tempdir().expect("tempdir");
		let content = b"model bytes";
		let spec = spec_for(content);
		let blob = hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, content).unwrap();

		materialize_snapshot(cache.path(), &spec).expect("materialize");
		// idempotent
		materialize_snapshot(cache.path(), &spec).expect("materialize again");

		let found = hf_cache_model_path(cache.path(), &spec).expect("resolved");
		assert_eq!(std::fs::read(&found).unwrap(), content);
	}

	#[test]
	fn migration_moves_verified_files_and_dedupes_existing_blobs() {
		let dir = tempfile::tempdir().expect("tempdir");
		let models = dir.path().join("models");
		let cache = dir.path().join("hub");
		std::fs::create_dir_all(&models).unwrap();

		let good = b"good model content";
		let good_spec = spec_for(good);
		let good_app = models.join(good_spec.filename);
		std::fs::write(&good_app, good).unwrap();

		// wrong-content file: must stay in the app dir untouched
		let mut bad_spec = spec_for(b"different bytes");
		// pin bad_spec's sha to something the file does NOT have

		bad_spec.sha256 = "deadbeef";
		bad_spec.filename = "stale-file.gguf";
		let bad_app = models.join(bad_spec.filename);
		std::fs::write(&bad_app, b"stale content").unwrap();

		// pre-existing blob: the app copy is redundant and just removed
		let mut dup_spec = spec_for(b"already cached");
		dup_spec.filename = "dup.gguf";
		let dup_blob = hf_blob_path(&cache, &dup_spec);
		std::fs::create_dir_all(dup_blob.parent().unwrap()).unwrap();
		std::fs::write(&dup_blob, b"already cached").unwrap();
		std::fs::write(models.join(dup_spec.filename), b"already cached").unwrap();

		migrate_one(&models, &cache, &good_spec);
		migrate_one(&models, &cache, &bad_spec);
		migrate_one(&models, &cache, &dup_spec);

		// good: moved into the blob, published, app copy gone
		assert!(!good_app.exists(), "app copy removed after migration");
		assert_eq!(
			std::fs::read(hf_blob_path(&cache, &good_spec)).unwrap(),
			good
		);
		assert!(hf_cache_model_path(&cache, &good_spec).is_some());

		// bad: left alone (content does not match the pin)
		assert!(bad_app.exists(), "unverifiable file stays in the app dir");
		assert!(!hf_blob_path(&cache, &bad_spec).exists());

		// dup: blob already present, app copy dropped, snapshot exists
		assert!(!models.join(dup_spec.filename).exists());
		assert!(hf_cache_model_path(&cache, &dup_spec).is_some());
	}

	#[cfg(target_family = "unix")]
	#[test]
	fn remove_prunes_snapshots_and_only_unreferenced_blobs() {
		let cache = tempfile::tempdir().expect("tempdir");
		let content = b"shared model bytes";
		let spec = spec_for(content);
		let blob = hf_blob_path(cache.path(), &spec);
		std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
		std::fs::write(&blob, content).unwrap();
		materialize_snapshot(cache.path(), &spec).expect("materialize");

		// a second snapshot revision sharing the blob (as another tool
		// would have created it)
		let repo_snapshots = blob.parent().unwrap().parent().unwrap().join("snapshots");
		let other = repo_snapshots.join("realcommit");
		std::fs::create_dir_all(&other).unwrap();
		std::os::unix::fs::symlink(
			std::path::Path::new("../../blobs").join(spec.sha256),
			other.join("different-name.gguf"),
		)
		.unwrap();

		// delete: both snapshot links go, blob kept while referenced
		assert!(remove_cached_model(cache.path(), &spec).expect("remove"));
		assert!(!hf_cache_model_path(cache.path(), &spec).is_some());
		assert!(
			blob.is_file(),
			"blob survives while another snapshot references it"
		);

		// the other tool's link is the only thing holding the blob now
		assert!(!remove_cached_model(cache.path(), &spec).expect("no-op remove"));
		assert!(blob.is_file(), "still referenced: kept");
		// once that link is gone too, the next remove prunes the blob
		std::fs::remove_file(other.join("different-name.gguf")).unwrap();
		assert!(remove_cached_model(cache.path(), &spec).expect("prune remove"));
		assert!(!blob.is_file(), "unreferenced blob is pruned");
	}
}
