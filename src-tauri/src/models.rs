use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::db::Db;
use crate::llm::LocalLlm;
use crate::stt::SttEngine;

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

/// AI-related settings resolved from the settings table.
#[derive(Debug, Clone)]
pub struct AiSettings {
	pub llm_mode: String,
	pub llm_model: String,
	pub stt_model: String,
	/// HuggingFace access token; sent with model downloads, where it
	/// avoids anonymous rate limits and can speed up large transfers.
	pub hf_token: String,
	pub ext_llm_base_url: String,
	pub ext_llm_api_key: String,
	pub ext_llm_model: String,
	pub ext_stt_base_url: String,
	pub ext_stt_api_key: String,
	pub ext_stt_model: String,
}

impl AiSettings {
	pub fn load(db: &Db) -> Self {
		let get = |k: &str| db.get_setting(k).unwrap_or_default();
		Self {
			llm_mode: {
				let m = get("ai_llm_mode");
				if m.is_empty() {
					"local".into()
				} else {
					m
				}
			},
			llm_model: {
				let m = get("ai_llm_model");
				if m.is_empty() {
					LLM_MODELS[0].id.to_string()
				} else {
					m
				}
			},
			stt_model: {
				let m = get("ai_stt_model");
				if m.is_empty() {
					STT_MODELS[0].id.to_string()
				} else {
					m
				}
			},
		hf_token: get("hf_token"),
		ext_llm_base_url: get("ext_llm_base_url"),
		ext_llm_api_key: get("ext_llm_api_key"),
		ext_llm_model: get("ext_llm_model"),
		ext_stt_base_url: get("ext_stt_base_url"),
		ext_stt_api_key: get("ext_stt_api_key"),
		ext_stt_model: get("ext_stt_model"),
	}
	}

	pub fn save(&self, db: &Db) {
		if let Err(e) = db.set_settings(&[
			("ai_llm_mode", self.llm_mode.clone()),
			("ai_llm_model", self.llm_model.clone()),
			("ai_stt_model", self.stt_model.clone()),
			("hf_token", self.hf_token.clone()),
			("ext_llm_base_url", self.ext_llm_base_url.clone()),
			("ext_llm_api_key", self.ext_llm_api_key.clone()),
			("ext_llm_model", self.ext_llm_model.clone()),
			("ext_stt_base_url", self.ext_stt_base_url.clone()),
			("ext_stt_api_key", self.ext_stt_api_key.clone()),
			("ext_stt_model", self.ext_stt_model.clone()),
		]) {
			log::error!("failed to save AI settings: {e}");
		}
	}
}

pub struct Runtime {
	pub backend: Option<Arc<llama_cpp_2::llama_backend::LlamaBackend>>,
	pub llm: Option<Arc<LocalLlm>>,
	pub stt: Option<Arc<SttEngine>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineStatus {
	pub state: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
}

impl EngineStatus {
	pub fn new(state: &str, model_id: Option<&str>, error: Option<&str>) -> Self {
		Self {
			state: state.into(),
			model_id: model_id.map(|s| s.into()),
			error: error.map(|s| s.into()),
		}
	}
}

pub struct AppState {
	pub db: Db,
	pub data_dir: PathBuf,
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
			runtime: std::sync::Mutex::new(Runtime {
				backend: None,
				llm: None,
				stt: None,
			}),
			llm_status: std::sync::Mutex::new(EngineStatus::new("missing", None, None)),
			stt_status: std::sync::Mutex::new(EngineStatus::new("missing", None, None)),
			generation_cancel: std::sync::Mutex::new(Arc::new(AtomicBool::new(false))),
			download_progress: std::sync::Mutex::new(std::collections::HashMap::new()),
			download_cancels: std::sync::Mutex::new(std::collections::HashMap::new()),
			llm_loading: AtomicBool::new(false),
			stt_loading: AtomicBool::new(false),
			quit_on_close: AtomicBool::new(false),
		}
	}

	pub fn models_dir(&self) -> PathBuf {
		self.data_dir.join("models")
	}

	pub fn model_path(&self, spec: &ModelSpec) -> PathBuf {
		self.models_dir().join(spec.filename)
	}

	pub fn is_model_downloaded(&self, spec: &ModelSpec) -> bool {
		self.model_path(spec).is_file()
	}

	pub fn emit_llm_status(&self, app: &AppHandle) {
		let status = lock(&self.llm_status).clone();
		let _ = app.emit("llm-status", status);
	}

	pub fn emit_stt_status(&self, app: &AppHandle) {
		let status = lock(&self.stt_status).clone();
		let _ = app.emit("stt-status", status);
	}

	/// Load the given LLM model file into the runtime. Blocking; call from a
	/// background thread.
	pub fn load_llm(&self, app: &AppHandle, spec: &ModelSpec) {
		// One load at a time: a second activate while the first is running
		// would mmap two multi-GB models simultaneously.
		if self.llm_loading.swap(true, Ordering::SeqCst) {
			log::warn!(
				"llm load already in progress, ignoring request for {}",
				spec.id
			);
			return;
		}
		{
			let mut s = lock(&self.llm_status);
			*s = EngineStatus::new("loading", Some(spec.id), None);
		}
		self.emit_llm_status(app);

		let path = self.model_path(spec);
		let result = (|| -> Result<LocalLlm, String> {
			// Drop the previous engine before loading the new file so peak
			// memory stays at one model instead of two.
			{
				let mut runtime = lock(&self.runtime);
				if runtime.backend.is_none() {
					let backend = llama_cpp_2::llama_backend::LlamaBackend::init()
						.map_err(|e| format!("failed to init llama backend: {e}"))?;
					runtime.backend = Some(Arc::new(backend));
				}
				runtime.llm = None;
			}
			let backend = lock(&self.runtime).backend.clone().unwrap();
			LocalLlm::load(backend, &path, spec.id)
		})();

		match result {
			Ok(engine) => {
				if path.is_file() {
					let mut runtime = lock(&self.runtime);
					runtime.llm = Some(Arc::new(engine));
					drop(runtime);
					let mut s = lock(&self.llm_status);
					*s = EngineStatus::new("ready", Some(spec.id), None);
				} else {
					// The model file was deleted while loading; don't
					// resurrect a deleted model in the runtime.
					log::warn!("{} was deleted while loading; not activating it", spec.id);
					let mut s = lock(&self.llm_status);
					*s = EngineStatus::new("missing", None, None);
				}
			}
			Err(e) => {
				log::error!("llm load failed: {e}");
				let mut s = lock(&self.llm_status);
				*s = EngineStatus::new("error", Some(spec.id), Some(&e));
			}
		}
		self.llm_loading.store(false, Ordering::SeqCst);
		self.emit_llm_status(app);
	}

	/// Load the given whisper model file. Blocking; call from a background thread.
	pub fn load_stt(&self, app: &AppHandle, spec: &ModelSpec) {
		if self.stt_loading.swap(true, Ordering::SeqCst) {
			log::warn!(
				"stt load already in progress, ignoring request for {}",
				spec.id
			);
			return;
		}
		{
			let mut s = lock(&self.stt_status);
			*s = EngineStatus::new("loading", Some(spec.id), None);
		}
		self.emit_stt_status(app);

		let path = self.model_path(spec);
		let result = {
			// Free the previous engine before loading the new file.
			lock(&self.runtime).stt = None;
			SttEngine::load(&path, spec.id)
		};

		match result {
			Ok(engine) => {
				if path.is_file() {
					let mut runtime = lock(&self.runtime);
					runtime.stt = Some(Arc::new(engine));
					drop(runtime);
					let mut s = lock(&self.stt_status);
					*s = EngineStatus::new("ready", Some(spec.id), None);
				} else {
					log::warn!("{} was deleted while loading; not activating it", spec.id);
					let mut s = lock(&self.stt_status);
					*s = EngineStatus::new("missing", None, None);
				}
			}
			Err(e) => {
				log::error!("stt load failed: {e}");
				let mut s = lock(&self.stt_status);
				*s = EngineStatus::new("error", Some(spec.id), Some(&e));
			}
		}
		self.stt_loading.store(false, Ordering::SeqCst);
		self.emit_stt_status(app);
	}
}

pub fn model_url(spec: &ModelSpec) -> String {
	format!(
		"https://huggingface.co/{}/resolve/main/{}",
		spec.repo, spec.filename
	)
}

/// Stream a model file to disk, reporting progress through `on_progress`
/// (percentage 0-100). Verifies the download completed fully before moving
/// it into place; the `.part` file is removed on any failure.
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

	let tmp = dest.with_extension("part");
	if tmp.exists() {
		tokio::fs::remove_file(&tmp)
			.await
			.map_err(|e| e.to_string())?;
	}

	let client = reqwest::Client::builder()
		.connect_timeout(std::time::Duration::from_secs(15))
		.build()
		.map_err(|e| e.to_string())?;
	let mut request = client.get(url).header("User-Agent", "brainstory-desktop/0.1");
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

	let total = response.content_length().unwrap_or(0);
	use futures_util::StreamExt;
	let mut stream = response.bytes_stream();
	let mut file = tokio::fs::File::create(&tmp)
		.await
		.map_err(|e| e.to_string())?;
	use tokio::io::AsyncWriteExt;
	// Hash chunks as they are written so verification costs no extra pass
	// over a multi-gigabyte file.
	let mut hasher = (!expected_sha256.is_empty()).then(Sha256::new);

	// Every failure path below removes the partial file, so a retry starts
	// clean instead of leaving gigabytes of junk behind.
	let outcome = async {
		let mut downloaded: u64 = 0;
		let mut last_report: u64 = 0;
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
					0.0
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
			let actual = format!("{:x}", hasher.finalize());
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
