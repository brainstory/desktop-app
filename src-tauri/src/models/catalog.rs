//! The pinned model catalog: what can be downloaded, from where,
//! with which integrity pins.

use serde::Serialize;

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
	/// Further files of the same repo the model needs beside `filename`
	/// (an ONNX model's decoder, preprocessor and vocabulary). Each is
	/// pinned like the main file, downloaded with it, and published next
	/// to it in the main file's snapshot directory, so the whole model
	/// loads from one folder. Empty for single-file (GGUF/ggml) models.
	pub companions: &'static [CompanionFile],
}

/// One extra pinned file of a multi-file model (see
/// [`ModelSpec::companions`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompanionFile {
	pub filename: &'static str,
	pub size_bytes: u64,
	pub sha256: &'static str,
}

impl ModelSpec {
	/// The model's companion files as specs of their own (same id, repo
	/// and kind; their own file, size and pin), so the download, cache
	/// and delete paths handle them with the same code as main files.
	pub fn companion_specs(&self) -> Vec<ModelSpec> {
		self.companions
			.iter()
			.map(|c| ModelSpec {
				filename: c.filename,
				size_bytes: c.size_bytes,
				sha256: c.sha256,
				companions: &[],
				..self.clone()
			})
			.collect()
	}

	/// Every file of the model: the main file, then its companions.
	pub fn all_file_specs(&self) -> Vec<ModelSpec> {
		let mut specs = vec![self.clone()];
		specs.extend(self.companion_specs());
		specs
	}

	/// Download size of the whole model, companions included.
	pub fn total_size_bytes(&self) -> u64 {
		self.size_bytes + self.companions.iter().map(|c| c.size_bytes).sum::<u64>()
	}

	/// Which local engine runs this speech model: an ONNX model is a
	/// Parakeet (ONNX Runtime) build, everything else whisper.cpp ggml.
	pub fn stt_backend(&self) -> SttBackend {
		if self.filename.ends_with(".onnx") {
			SttBackend::Parakeet
		} else {
			SttBackend::Whisper
		}
	}
}

/// The local engine family a speech-to-text catalog model runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttBackend {
	Whisper,
	Parakeet,
}

/// Known-good open model builds. The local LLM catalog defaults to Google's
/// Gemma 4 edge models (QAT quantized, runnable on integrated GPUs / Apple
/// Silicon) with Qwen3.5 4B and MiniCPM5 as alternatives; STT ships
/// whisper.cpp ggml builds and NVIDIA's Parakeet TDT v3 (int8 ONNX).
pub const LLM_MODELS: [ModelSpec; 4] = [
	ModelSpec {
		id: "gemma-4-E2B-qat",
		kind: ModelKind::Llm,
		label: "Gemma 4 E2B (light)",
		description: "Google Gemma 4 E2B, compressed to run on a laptop (~3.3 GB). The balanced default.",
		repo: "google/gemma-4-E2B-it-qat-q4_0-gguf",
		filename: "gemma-4-E2B_q4_0-it.gguf",
		size_bytes: 3_349_516_256,
		sha256: "fa401b55b07ee70a54c6dae3903c783a6e65064312529ea57175cb5f8dec6634",
		companions: &[],
	},
	ModelSpec {
		id: "gemma-4-E4B",
		kind: ModelKind::Llm,
		label: "Gemma 4 E4B",
		description: "The larger Gemma 4 (~5.2 GB): noticeably better writing, needs a bit more memory.",
		repo: "google/gemma-4-E4B-it-qat-q4_0-gguf",
		filename: "gemma-4-E4B_q4_0-it.gguf",
		size_bytes: 5_154_941_280,
		sha256: "676c35070db6dbe52f93e9c864ee0fba4eddea94b9c875d9cb10daff453fbaee",
		companions: &[],
	},
	ModelSpec {
		id: "qwen35-4b",
		kind: ModelKind::Llm,
		label: "Qwen3.5 4B",
		description: "Alibaba Qwen3.5 4B (~2.7 GB): strong reasoning for its size and a smaller download than Gemma 4 E2B.",
		repo: "unsloth/Qwen3.5-4B-GGUF",
		filename: "Qwen3.5-4B-Q4_K_M.gguf",
		size_bytes: 2_740_937_888,
		sha256: "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
		companions: &[],
	},
	ModelSpec {
		id: "minicpm5-2b",
		kind: ModelKind::Llm,
		label: "MiniCPM5 2B",
		description: "A compact 2B model (~1.6 GB). The smallest and fastest option; good for lighter machines.",
		repo: "openbmb/MiniCPM5-2B-GGUF",
		filename: "MiniCPM5-2B-Q4_K_M.gguf",
		size_bytes: 1_561_318_368,
		sha256: "ec2d5801640099e97d8d7e8003ad4d81f336e757811f03a26173dddf386602fd",
		companions: &[],
	},
];

/// The STT model a fresh install selects. Named, not positional: the
/// download list leads with Parakeet, and the default must not silently
/// follow the list order.
pub const DEFAULT_STT_MODEL: &str = "whisper-base";

/// STT models no longer offered for download, kept in the catalog so an
/// install that already downloaded or selected one keeps working (and
/// can still delete it): whisper tiny's accuracy is too rough to
/// recommend next to the models worth having.
const RETIRED_STT_MODELS: [&str; 1] = ["whisper-tiny-en"];

/// True when the model should appear in the settings download list.
pub fn is_offered_for_download(spec: &ModelSpec) -> bool {
	!RETIRED_STT_MODELS.contains(&spec.id)
}

pub const STT_MODELS: [ModelSpec; 7] = [
	ModelSpec {
		id: "parakeet-tdt-0.6b-v3",
		kind: ModelKind::Stt,
		label: "Parakeet TDT v3 (25 languages)",
		description: "NVIDIA Parakeet (~670 MB): more accurate than Whisper large v3 turbo and several times faster. English and 24 European languages.",
		repo: "istupakov/parakeet-tdt-0.6b-v3-onnx",
		filename: "encoder-model.int8.onnx",
		size_bytes: 652_183_999,
		sha256: "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09",
		companions: &[
			CompanionFile {
				filename: "decoder_joint-model.int8.onnx",
				size_bytes: 18_202_004,
				sha256: "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70",
			},
			CompanionFile {
				filename: "nemo128.onnx",
				size_bytes: 139_764,
				sha256: "a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f",
			},
			CompanionFile {
				filename: "vocab.txt",
				size_bytes: 93_939,
				sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
			},
		],
	},
	ModelSpec {
		id: "whisper-base",
		kind: ModelKind::Stt,
		label: "Whisper base (multilingual)",
		description: "Everyday transcription in any language (~148 MB): fast, accurate enough.",
		repo: "ggerganov/whisper.cpp",
		filename: "ggml-base.bin",
		size_bytes: 147_951_465,
		sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
		companions: &[],
	},
	ModelSpec {
		id: "whisper-small",
		kind: ModelKind::Stt,
		label: "Whisper small (multilingual)",
		description: "More accurate transcription in any language (~488 MB), still quick.",
		repo: "ggerganov/whisper.cpp",
		filename: "ggml-small.bin",
		size_bytes: 487_601_967,
		sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
		companions: &[],
	},
	ModelSpec {
		id: "whisper-base-en",
		kind: ModelKind::Stt,
		label: "Whisper base (English)",
		description: "Good everyday transcription (~148 MB): fast, accurate enough.",
		repo: "ggerganov/whisper.cpp",
		filename: "ggml-base.en.bin",
		size_bytes: 147_964_211,
		sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
		companions: &[],
	},
	ModelSpec {
		id: "whisper-small-en",
		kind: ModelKind::Stt,
		label: "Whisper small (English)",
		description: "More accurate transcription (~488 MB), still quick.",
		repo: "ggerganov/whisper.cpp",
		filename: "ggml-small.en.bin",
		size_bytes: 487_614_201,
		sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
		companions: &[],
	},
	ModelSpec {
		id: "whisper-tiny-en",
		kind: ModelKind::Stt,
		label: "Whisper tiny (English)",
		description: "The fastest and smallest model (~78 MB); accuracy is rough.",
		repo: "ggerganov/whisper.cpp",
		filename: "ggml-tiny.en.bin",
		size_bytes: 77_704_715,
		sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
		companions: &[],
	},
	ModelSpec {
		id: "whisper-large-v3-turbo",
		kind: ModelKind::Stt,
		label: "Whisper large v3 turbo",
		description: "The most accurate option (~1.6 GB).",
		repo: "ggerganov/whisper.cpp",
		filename: "ggml-large-v3-turbo.bin",
		size_bytes: 1_624_555_275,
		sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
		companions: &[],
	},
];

pub fn find_model(id: &str, kind: ModelKind) -> Option<&'static ModelSpec> {
	let list: &[ModelSpec] = match kind {
		ModelKind::Llm => &LLM_MODELS,
		ModelKind::Stt => &STT_MODELS,
	};
	list.iter().find(|m| m.id == id)
}

/// Conservative KV-cache bytes/token for models not in the catalog; see
/// [`kv_bytes_per_token`].
const UNKNOWN_LLM_KV_BYTES_PER_TOKEN: u64 = 524_288;

/// Approximate KV-cache memory per token (bytes) for a local LLM, for
/// the context-window memory hint in settings. llama.cpp keeps K and V
/// in f16 by default, so one token costs
/// 2 (K and V) × layers × kv_heads × head_dim × 2 bytes.
/// The per-model constants are each catalog model's published
/// architecture (the config.json of its pinned HF repo / its GGUF
/// readme) - documented constants, never probed at runtime. An unknown
/// model gets a documented fallback that over-estimates rather than
/// under-estimates.
pub fn kv_bytes_per_token(spec: &ModelSpec) -> u64 {
	match spec.id {
		// Gemma 4 E2B (google/gemma-4-E2B-it-qat-q4_0-gguf): the edge
		// architecture publishes 23 layers, 4 KV heads (GQA over 8 query
		// heads), head_dim 256:
		//   2 × 23 × 4 × 256 × 2 = 94_208 bytes/token
		"gemma-4-E2B-qat" => 94_208,
		// Gemma 4 E4B (google/gemma-4-E4B-it-qat-q4_0-gguf): 35 layers,
		// 4 KV heads, head_dim 256:
		//   2 × 35 × 4 × 256 × 2 = 143_360 bytes/token
		"gemma-4-E4B" => 143_360,
		// Qwen3.5 4B (Qwen/Qwen3.5-4B config.json): a hybrid of 32
		// layers where only every 4th is full attention (8 layers, 4 KV
		// heads, head_dim 256); the other 24 are Gated DeltaNet linear
		// attention, whose recurrent state is a fixed size independent
		// of the context length, so it costs nothing per token:
		//   2 × 8 × 4 × 256 × 2 = 32_768 bytes/token
		"qwen35-4b" => 32_768,
		// MiniCPM5 2B (openbmb/MiniCPM5-2B-GGUF): Llama-style 40 layers,
		// 4 KV heads, head_dim 128:
		//   2 × 40 × 4 × 128 × 2 = 81_920 bytes/token
		"minicpm5-2b" => 81_920,
		// Unknown model: assume the most KV-hungry architecture a user
		// can realistically run locally - a 32-layer, full-attention
		// (32 heads, no GQA), head_dim-128 7B-class model:
		//   2 × 32 × 32 × 128 × 2 = 524_288 bytes/token
		// Dense attention costs several times the GQA edge models above,
		// so the fallback is always ≥ every catalog entry: the hint errs
		// towards warning the user, never towards under-sizing.
		_ => UNKNOWN_LLM_KV_BYTES_PER_TOKEN,
	}
}

#[cfg(test)]
mod tests {
	use super::{kv_bytes_per_token, ModelKind, ModelSpec, LLM_MODELS, STT_MODELS};

	#[test]
	fn kv_bytes_per_token_is_keyed_by_the_catalog_entry() {
		for spec in LLM_MODELS.iter() {
			// the inline derivation in kv_bytes_per_token, restated here
			// so a constant that drifts from its architecture fails
			let expected: u64 = match spec.id {
				"gemma-4-E2B-qat" => 2 * 23 * 4 * 256 * 2,
				"gemma-4-E4B" => 2 * 35 * 4 * 256 * 2,
				"qwen35-4b" => 2 * 8 * 4 * 256 * 2,
				"minicpm5-2b" => 2 * 40 * 4 * 128 * 2,
				other => panic!("document the KV cost of catalog model {other}"),
			};
			assert_eq!(
				kv_bytes_per_token(spec),
				expected,
				"{} must match its documented architecture",
				spec.id
			);
		}
	}

	#[test]
	fn unknown_models_get_a_conservative_over_estimate() {
		let unknown = ModelSpec {
			id: "some-finetune-13b",
			kind: ModelKind::Llm,
			label: "unknown",
			description: "unknown",
			repo: "unknown",
			filename: "unknown.gguf",
			size_bytes: 0,
			sha256: "",
			companions: &[],
		};
		let fallback = kv_bytes_per_token(&unknown);
		assert_eq!(fallback, 2 * 32 * 32 * 128 * 2);
		for spec in LLM_MODELS.iter() {
			assert!(
				fallback >= kv_bytes_per_token(spec),
				"the fallback must never under-estimate {}",
				spec.id
			);
		}
		// whisper rows are not LLMs; they too get the documented fallback
		let whisper = super::find_model("whisper-base", ModelKind::Stt).expect("catalog");
		assert_eq!(kv_bytes_per_token(whisper), fallback);
	}
	#[test]
	fn the_stt_list_leads_with_parakeet_and_the_default_stays_pinned() {
		assert_eq!(
			STT_MODELS[0].id, "parakeet-tdt-0.6b-v3",
			"the list leads with Parakeet"
		);
		// the default must NOT follow the list order: it was STT_MODELS[0]
		// before Parakeet moved first, and fresh installs keep whisper base
		assert_eq!(super::DEFAULT_STT_MODEL, "whisper-base");
		assert!(
			super::find_model(super::DEFAULT_STT_MODEL, ModelKind::Stt).is_some(),
			"the default names a catalog model"
		);
	}

	#[test]
	fn retired_models_stay_in_the_catalog_but_leave_the_download_list() {
		let tiny = super::find_model("whisper-tiny-en", ModelKind::Stt)
			.expect("kept for existing installs");
		assert!(!super::is_offered_for_download(tiny));
		for spec in STT_MODELS
			.iter()
			.filter(|spec| spec.id != "whisper-tiny-en")
		{
			assert!(
				super::is_offered_for_download(spec),
				"{} stays offered",
				spec.id
			);
		}
	}

	#[test]
	fn companions_are_pinned_files_of_the_same_model() {
		let parakeet = super::find_model("parakeet-tdt-0.6b-v3", ModelKind::Stt).expect("catalog");
		assert_eq!(parakeet.stt_backend(), super::SttBackend::Parakeet);
		let files = parakeet.all_file_specs();
		assert_eq!(
			files.iter().map(|f| f.filename).collect::<Vec<_>>(),
			[
				"encoder-model.int8.onnx",
				"decoder_joint-model.int8.onnx",
				"nemo128.onnx",
				"vocab.txt"
			]
		);
		for file in &files {
			assert_eq!(
				(file.id, file.repo, file.kind),
				(parakeet.id, parakeet.repo, parakeet.kind)
			);
			assert_eq!(file.sha256.len(), 64, "{} is pinned", file.filename);
		}
		// a companion is a plain file: it has no companions of its own
		assert!(files[1..].iter().all(|f| f.companions.is_empty()));
		assert_eq!(
			parakeet.total_size_bytes(),
			files.iter().map(|f| f.size_bytes).sum::<u64>()
		);
		// single-file models are unchanged: whisper ggml, no companions
		for spec in STT_MODELS.iter().filter(|s| s.id.starts_with("whisper-")) {
			assert_eq!(spec.stt_backend(), super::SttBackend::Whisper);
			assert_eq!(spec.all_file_specs().len(), 1);
			assert_eq!(spec.total_size_bytes(), spec.size_bytes);
		}
	}
}
