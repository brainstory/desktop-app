use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::llama_backend::LlamaBackend;

use crate::types::ChatMessage;

/// Generation context size. Transcripts are conversational; 16k tokens
/// comfortably fits a long session plus the result document.
const N_CTX: u32 = 16384;
const MAX_NEW_TOKENS_RESPONSE: u32 = 1024;
const MAX_NEW_TOKENS_RESULT: u32 = 4096;

#[allow(dead_code)]
pub struct LocalLlm {
	backend: Arc<LlamaBackend>,
	model: Arc<LlamaModel>,
	pub model_id: String,
	/// general.architecture from the GGUF metadata
	architecture: String,
}

impl LocalLlm {
	pub fn load(backend: Arc<LlamaBackend>, path: &Path, model_id: &str) -> Result<Self, String> {
		// Offload every layer to the GPU when a backend (Metal/Vulkan) exists;
		// llama.cpp falls back to CPU compute automatically otherwise.
		let params = LlamaModelParams::default().with_n_gpu_layers(u32::MAX);
		let model = LlamaModel::load_from_file(&backend, path, &params)
			.map_err(|e| format!("failed to load model {path:?}: {e}"))?;
		let architecture = model
			.meta_val_str("general.architecture")
			.unwrap_or_default();
		Ok(Self { backend, model: Arc::new(model), model_id: model_id.to_string(), architecture })
	}

	fn apply_llama_template(
		&self,
		template: &LlamaChatTemplate,
		chat: &[LlamaChatMessage],
	) -> Result<String, String> {
		self.model
			.apply_chat_template(template, chat, true)
			.map_err(|e| format!("failed to apply chat template: {e}"))
	}

	/// Manual prompt format matching the model's native chat markers. Used
	/// when llama.cpp's built-in template applier can't handle the model's
	/// (e.g. the Gemma 4 canonical template, which its minja subset rejects).
	fn manual_prompt(&self, system: &str, messages: &[ChatMessage]) -> String {
		if self.architecture.starts_with("gemma") {
			// Gemma 4: <bos><|turn>role\ncontent<turn|>... ending with an
			// open model turn. The system turn (if any) comes first.
			let mut prompt = String::from("<bos>");
			if !system.trim().is_empty() {
				prompt.push_str(&format!("<|turn>system\n{}<turn|>\n", system.trim()));
			}
			for msg in messages {
				let role = if msg.role == "assistant" { "model" } else { "user" };
				let content = if role == "user" { msg.content.trim() } else { msg.content.as_str() };
				prompt.push_str(&format!("<|turn>{role}\n{content}<turn|>\n"));
			}
			prompt.push_str("<|turn>model\n");
			prompt
		} else {
			// last resort: plain concatenation
			let mut prompt = String::new();
			if !system.trim().is_empty() {
				prompt.push_str(system.trim());
				prompt.push_str("\n\n");
			}
			for msg in messages {
				let role = if msg.role == "assistant" { "Assistant" } else { "User" };
				prompt.push_str(&format!("{role}: {}\n", msg.content));
			}
			prompt.push_str("Assistant:");
			prompt
		}
	}

	fn apply_template(&self, system: &str, messages: &[ChatMessage]) -> Result<String, String> {
		if let Ok(template) = self.model.chat_template(None) {
			let mut chat: Vec<LlamaChatMessage> = Vec::new();
			if !system.trim().is_empty() {
				chat.push(
					LlamaChatMessage::new("system".to_string(), system.to_string())
						.map_err(|e| e.to_string())?,
				);
			}
			for msg in messages {
				let role = if msg.role == "assistant" { "assistant" } else { "user" };
				chat.push(
					LlamaChatMessage::new(role.to_string(), msg.content.clone())
						.map_err(|e| e.to_string())?,
				);
			}
			if let Ok(prompt) = self.apply_llama_template(&template, &chat) {
				return Ok(prompt);
			}
			log::warn!(
				"built-in chat template failed, falling back to manual {} format",
				self.architecture
			);
		}
		Ok(self.manual_prompt(system, messages))
	}

	fn count_tokens(&self, prompt: &str) -> Result<usize, String> {
		Ok(self.model.str_to_token(prompt, AddBos::Never).map_err(|e| e.to_string())?.len())
	}

	/// Build the final prompt, dropping older middle messages until it fits
	/// into the context window with room for `max_new_tokens`.
	fn build_prompt(
		&self,
		system: &str,
		messages: &[ChatMessage],
		max_new_tokens: u32,
	) -> Result<String, String> {
		let budget = (N_CTX as usize).saturating_sub(max_new_tokens as usize + 64);
		let mut msgs: Vec<ChatMessage> = messages.to_vec();
		let mut prompt = self.apply_template(system, &msgs)?;
		let mut n_tokens = self.count_tokens(&prompt)?;

		// Drop the second-oldest message repeatedly (keeps the opening
		// assistant framing and the most recent context).
		while n_tokens > budget && msgs.len() > 2 {
			msgs.remove(1);
			prompt = self.apply_template(system, &msgs)?;
			n_tokens = self.count_tokens(&prompt)?;
		}
		Ok(prompt)
	}

	/// Run generation, streaming pieces through `on_chunk`. Blocking and
	/// CPU/GPU heavy; run on a dedicated thread.
	pub fn generate(
		&self,
		system: &str,
		messages: &[ChatMessage],
		summarize: bool,
		cancel: &AtomicBool,
		mut on_chunk: impl FnMut(String),
	) -> Result<String, String> {
		let max_new = if summarize { MAX_NEW_TOKENS_RESULT } else { MAX_NEW_TOKENS_RESPONSE };
		let prompt = self.build_prompt(system, messages, max_new)?;
		let tokens = self.model.str_to_token(&prompt, AddBos::Never).map_err(|e| e.to_string())?;
		if tokens.is_empty() {
			return Err("empty prompt".into());
		}

		// The context borrows the model, so it lives only within this call.
		let ctx_params = LlamaContextParams::default()
			.with_n_ctx(NonZeroU32::new(N_CTX))
			.with_n_batch(tokens.len().max(512) as u32);
		let mut ctx = self
			.model
			.new_context(&self.backend, ctx_params)
			.map_err(|e| format!("failed to create context: {e}"))?;

		let mut batch = LlamaBatch::new(tokens.len().max(512), 1);
		for (i, token) in tokens.iter().enumerate() {
			batch.add(*token, i as i32, &[0], i + 1 == tokens.len()).map_err(|e| e.to_string())?;
		}
		ctx.decode(&mut batch).map_err(|e| format!("prompt decode failed: {e}"))?;

		// Gemma-recommended sampling: top_k 64, top_p 0.95. Summaries use a
		// lower temperature for more deterministic structure.
		let temperature = if summarize { 0.35 } else { 0.7 };
		let seed = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map(|d| d.subsec_nanos())
			.unwrap_or(42);
		let mut sampler = LlamaSampler::chain(
			[
				LlamaSampler::top_k(64),
				LlamaSampler::top_p(0.95, 1),
				LlamaSampler::temp(temperature),
				LlamaSampler::dist(seed),
			],
			true,
		);

		let mut decoder = encoding_rs::UTF_8.new_decoder();
		let mut output = String::new();
		let mut generated: u32 = 0;
		let mut pos = tokens.len() as i32;

		loop {
			if cancel.load(Ordering::Relaxed) {
				return Err("generation cancelled".into());
			}
			let token = sampler.sample(&ctx, -1);
			if self.model.is_eog_token(token) {
				break;
			}
			let piece = match self
				.model
				.token_to_piece(token, &mut decoder, false, None)
			{
				Ok(piece) => piece,
				// a token with no text piece is not an error; feed it back
				Err(llama_cpp_2::TokenToStringError::UnknownTokenType) => {
					String::new()
				}
				Err(e) => return Err(e.to_string()),
			};
			if !piece.is_empty() {
				on_chunk(piece.clone());
				output.push_str(&piece);
			}
			generated += 1;
			if generated >= max_new {
				break;
			}

			batch.clear();
			batch.add(token, pos, &[0], true).map_err(|e| e.to_string())?;
			ctx.decode(&mut batch).map_err(|e| format!("decode failed: {e}"))?;
			pos += 1;
		}

		Ok(output)
	}
}

/// OpenAI-compatible chat completion endpoint (Ollama, llama.cpp server,
/// LM Studio, vLLM, ...). Streams tokens over SSE.
pub struct ExternalLlm {
	pub base_url: String,
	pub api_key: String,
	pub model: String,
	client: reqwest::Client,
}

impl ExternalLlm {
	pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
		Self {
			base_url: base_url.trim_end_matches('/').to_string(),
			api_key: api_key.to_string(),
			model: model.to_string(),
			client: reqwest::Client::new(),
		}
	}

	fn completions_url(&self) -> String {
		if self.base_url.ends_with("/v1") {
			format!("{}/chat/completions", self.base_url)
		} else {
			format!("{}/v1/chat/completions", self.base_url)
		}
	}

	pub async fn generate(
		&self,
		system: &str,
		messages: &[ChatMessage],
		cancel: &AtomicBool,
		mut on_chunk: impl FnMut(String) + Send,
	) -> Result<String, String> {
		let mut body_messages = vec![];
		if !system.trim().is_empty() {
			body_messages.push(serde_json::json!({"role": "system", "content": system}));
		}
		for msg in messages {
			body_messages.push(serde_json::json!({"role": msg.role, "content": msg.content}));
		}

		let mut request = self
			.client
			.post(self.completions_url())
			.json(&serde_json::json!({
				"model": self.model,
				"messages": body_messages,
				"stream": true,
				"temperature": 0.7,
			}));
		if !self.api_key.is_empty() {
			request = request.bearer_auth(&self.api_key);
		}

		let response = request.send().await.map_err(|e| format!("request failed: {e}"))?;
		if !response.status().is_success() {
			let status = response.status();
			let body = response.text().await.unwrap_or_default();
			return Err(map_provider_error(status.as_u16(), &body));
		}

		use futures_util::StreamExt;
		let mut stream = response.bytes_stream();
		let mut buffer: Vec<u8> = Vec::new();
		let mut output = String::new();

		loop {
			if cancel.load(Ordering::Relaxed) {
				return Err("generation cancelled".into());
			}
			let chunk = match stream.next().await {
				Some(Ok(c)) => c,
				Some(Err(e)) => return Err(format!("stream interrupted: {e}")),
				None => break,
			};
			buffer.extend_from_slice(&chunk);

			while let Some(pos) = find_double_newline(&buffer) {
				let line_bytes: Vec<u8> = buffer.drain(..pos).collect();
				let line = String::from_utf8_lossy(&line_bytes);
				let line = line.trim();
				if let Some(data) = line.strip_prefix("data:") {
					let data = data.trim();
					if data == "[DONE]" {
						return Ok(output);
					}
					if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
						if let Some(delta) = value["choices"][0]["delta"]["content"].as_str() {
							if !delta.is_empty() {
								output.push_str(delta);
								on_chunk(delta.to_string());
							}
						}
						if let Some(err) = value["error"]["message"].as_str() {
							return Err(map_provider_error(0, err));
						}
					}
				}
			}
		}
		Ok(output)
	}
}

fn find_double_newline(buffer: &[u8]) -> Option<usize> {
	buffer.windows(2).position(|w| w == b"\n\n").map(|p| p + 2)
}

/// Map provider errors onto the brainstory protocol. HTTP 469 was the
/// original "inappropriate input" signal that the frontend knows how to
/// surface as a resend request.
fn map_provider_error(status: u16, body: &str) -> String {
	let lower = body.to_lowercase();
	if status == 400 && (lower.contains("content_filter") || lower.contains("content_policy")) {
		return "HttpError 469: Inappropriate input".into();
	}
	if status == 401 || status == 403 {
		return format!("external endpoint rejected credentials ({status})");
	}
	if status != 0 {
		return format!("external endpoint error ({status}): {}", truncate_body(body));
	}
	truncate_body(body)
}

fn truncate_body(body: &str) -> String {
	if body.len() > 300 {
		format!("{}...", &body[..300])
	} else {
		body.to_string()
	}
}
