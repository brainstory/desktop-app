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
		assert_eq!(kv_bytes_per_token(&STT_MODELS[0]), fallback);
	}
}
