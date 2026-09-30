use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;

use crate::llm::ExternalLlm;
use crate::models::{AiSettings, AppState};
use crate::prompts::{ChatType, PromptRequest};
use crate::stt;
use crate::types::{ChatMessage, StreamEvent};

/// Cancel any in-flight generation and return the fresh token for this one.
/// The whole swap happens under one lock so two overlapping generations can
/// never end up with a token that `cancel_generation` can't reach.
fn take_cancel_token(state: &AppState) -> Arc<AtomicBool> {
	let token = Arc::new(AtomicBool::new(false));
	let mut current = state
		.generation_cancel
		.lock()
		.unwrap_or_else(|e| e.into_inner());
	let old = std::mem::replace(&mut *current, token.clone());
	old.store(true, Ordering::Relaxed);
	token
}

#[tauri::command]
pub async fn transcribe(
	state: State<'_, AppState>,
	request: tauri::ipc::Request<'_>,
) -> Result<serde_json::Value, String> {
	let bytes: Vec<u8> = match request.body() {
		tauri::ipc::InvokeBody::Raw(raw) => raw.to_vec(),
		_ => return Err("expected raw audio bytes".into()),
	};
	if bytes.is_empty() {
		return Err("no audio received".into());
	}
	// 300 s of 16 kHz mono 16-bit WAV is ~9.6 MB and the recorder stops at
	// 4 minutes; anything larger is a bug or abuse. The decoder
	// materializes several times the input size, so refuse instead of
	// risking an OOM.
	const MAX_TRANSCRIBE_BYTES: usize = 32 * 1024 * 1024;
	if bytes.len() > MAX_TRANSCRIBE_BYTES {
		return Err(format!(
			"audio capture too large ({} MB, limit 32 MB)",
			bytes.len() / (1024 * 1024)
		));
	}

	let settings = AiSettings::load(&state.db);
	// STT offload rule: if an external STT endpoint is configured, use it;
	// otherwise the engine setting picks Apple Speech or local whisper.
	if !settings.ext_stt_base_url.is_empty() {
		let transcript = stt::transcribe_external(
			&settings.ext_stt_base_url,
			&settings.ext_stt_api_key,
			&settings.ext_stt_model,
			bytes,
		)
		.await?;
		return Ok(serde_json::json!({ "transcript": transcript }));
	}

	// Apple Speech: tried whenever auto/apple is set AND the engine
	// exists on this system (no pointless clone + bridge call otherwise).
	// Auto falls back to whisper on any failure; an explicit Apple choice
	// on a supported system surfaces real failures (permission prompts,
	// asset problems) instead of hiding them behind whisper.
	if crate::apple::speech_available()
		&& matches!(
			settings.stt_engine,
			crate::models::SpeechEngine::Apple | crate::models::SpeechEngine::Auto
		) {
		let apple_bytes = bytes.clone();
		let locale = settings.stt_language.clone();
		let result = tauri::async_runtime::spawn_blocking(move || {
			crate::stt_apple::transcribe(&apple_bytes, &locale)
		})
		.await
		.map_err(|e| e.to_string());
		match result {
			Ok(Ok(transcript)) => return Ok(serde_json::json!({ "transcript": transcript })),
			Ok(Err(e)) => {
				let explicit_supported = settings.stt_engine == crate::models::SpeechEngine::Apple
					&& crate::apple::speech_available();
				if explicit_supported {
					return Err(e);
				}
				log::warn!("Apple Speech unavailable ({e}); falling back to whisper");
			}
			Err(e) => log::warn!("Apple Speech task failed: {e}; falling back to whisper"),
		}
	}

	let engine = {
		let runtime = state.runtime.lock().unwrap_or_else(|e| e.into_inner());
		runtime.stt.clone()
	};
	let engine = engine.ok_or_else(|| {
		"Speech model not downloaded yet. Open Settings > AI Models to download one.".to_string()
	})?;

	// WAV decoding of a multi-minute recording is CPU work too; keep it off
	// the async runtime alongside the whisper inference. The configured
	// language rides along so multilingual models honor it.
	let language = settings.stt_language.clone();
	let transcript = tauri::async_runtime::spawn_blocking(move || {
		let samples = stt::wav_to_samples(&bytes)?;
		engine.transcribe(&samples, &language)
	})
	.await
	.map_err(|e| e.to_string())??;

	Ok(serde_json::json!({ "transcript": transcript }))
}

/// The fields every generation command shares, resolved from the same
/// positional Tauri args, plus the per-command streaming plumbing.
/// Which stream event the sink is being asked to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmitKind {
	Status,
	Chunk,
	Cumulative,
}

impl EmitKind {
	fn as_str(self) -> &'static str {
		match self {
			EmitKind::Status => "status",
			EmitKind::Chunk => "chunk",
			EmitKind::Cumulative => "cumulative",
		}
	}
}

#[derive(Debug, Clone)]
struct GenerateParams {
	messages: Vec<ChatMessage>,
	summarize: bool,
	chat_type: Option<String>,
	react_to: Option<String>,
	react_to_author: Option<String>,
	react_to_is_current_user: bool,
	structured_feedback: bool,
}

impl GenerateParams {
	/// Resolve the prompt request; a react_to payload implies the
	/// feedback flow even if the frontend didn't label the chat type.
	fn into_request(self) -> PromptRequest {
		let mut request = PromptRequest {
			chat_type: ChatType::parse(self.chat_type.as_deref()),
			messages: self.messages,
			summarize: self.summarize,
			react_to: self.react_to,
			react_to_author: self.react_to_author,
			react_to_is_current_user: self.react_to_is_current_user,
			structured_feedback: self.structured_feedback,
		};
		if request.react_to.is_some() && request.chat_type == ChatType::Original {
			request.chat_type = ChatType::Feedback;
		}
		request
	}
}

/// Everything after the prompt is built: run the (optionally streaming)
/// generation and produce the shared response payload, including the
/// second structured-JSON pass for the standard feedback flow.
/// `on_event` is called with "status"/"chunk"/"cumulative" events; pass a
/// no-op sink for the non-streaming command.
async fn generate_and_respond<F>(
	state: &State<'_, AppState>,
	request: &PromptRequest,
	user_messages: &[ChatMessage],
	// second_pass: run the structured-JSON second pass (streaming only)
	second_pass: bool,
	// on_event returns false when the channel is dead: cancel generation
	on_event: F,
) -> Result<serde_json::Value, String>
where
	F: Fn(EmitKind, &str) -> bool + Send + Sync + Clone + 'static,
{
	let system = request.system_prompt();
	let word_count: usize = user_messages
		.iter()
		.map(|m| m.content.split_whitespace().count())
		.sum();
	let cancel = take_cancel_token(state);
	let settings = AiSettings::load(&state.db);

	on_event(EmitKind::Status, "thinking...");

	// If the webview went away (page reloaded mid-stream), stop generating
	// instead of burning CPU on output nobody will see.
	let cancel_for_writer = cancel.clone();
	let emit = on_event.clone();
	let mut channel_dead = false;
	let channel_writer = move |piece: String| {
		if channel_dead {
			return;
		}
		if !emit(EmitKind::Chunk, &piece) {
			channel_dead = true;
			cancel_for_writer.store(true, Ordering::Relaxed);
		}
	};
	let (output, prompt_tokens) = run_generation(
		state,
		&settings,
		system,
		user_messages,
		request.summarize,
		cancel.clone(),
		channel_writer,
	)
	.await?;

	on_event(EmitKind::Cumulative, &output);

	// Structured feedback: either the request itself asked for JSON, or
	// this is the standard feedback flow, in which case run a second pass
	// to also produce the structured JSON feedback document. (The
	// non-streaming command intentionally skips the second pass, matching
	// its previous behaviour.)
	let mut structured = None;
	if request.summarize && request.structured_feedback {
		structured = extract_feedback_json(&output);
	} else if second_pass && request.summarize && request.chat_type == ChatType::Feedback {
		let json_system = crate::prompts::FEEDBACK_JSON_RESULT_SYSTEM.to_string();
		let json_output = run_generation(
			state,
			&settings,
			json_system,
			user_messages,
			true,
			cancel,
			|_| {},
		)
		.await;
		match json_output {
			Ok((json_output, _)) => structured = extract_feedback_json(&json_output),
			// log instead of swallowing: a silently missing structured
			// document is invisible in support logs
			Err(e) => log::error!("structured feedback pass failed: {e}"),
		}
	}

	Ok(serde_json::json!({
		"response": output,
		"request_message_tokens": prompt_tokens,
		"request_word_count": word_count,
		"structured_result": structured,
	}))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn generate_response(
	state: State<'_, AppState>,
	messages: Vec<ChatMessage>,
	summarize: Option<bool>,
	chat_type: Option<String>,
	react_to: Option<String>,
	react_to_author: Option<String>,
	react_to_is_current_user: Option<bool>,
	structured_feedback: Option<bool>,
) -> Result<serde_json::Value, String> {
	let request = GenerateParams {
		messages,
		summarize: summarize.unwrap_or(false),
		chat_type,
		react_to,
		react_to_author,
		react_to_is_current_user: react_to_is_current_user.unwrap_or(false),
		structured_feedback: structured_feedback.unwrap_or(false),
	}
	.into_request();
	let user_messages = request.user_messages();
	generate_and_respond(&state, &request, &user_messages, false, |_, _| true).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn generate_streaming_response(
	state: State<'_, AppState>,
	messages: Vec<ChatMessage>,
	summarize: Option<bool>,
	chat_type: Option<String>,
	react_to: Option<String>,
	react_to_author: Option<String>,
	react_to_is_current_user: Option<bool>,
	structured_feedback: Option<bool>,
	on_event: tauri::ipc::Channel<StreamEvent>,
) -> Result<serde_json::Value, String> {
	let request = GenerateParams {
		messages,
		summarize: summarize.unwrap_or(false),
		chat_type,
		react_to,
		react_to_author,
		react_to_is_current_user: react_to_is_current_user.unwrap_or(false),
		structured_feedback: structured_feedback.unwrap_or(false),
	}
	.into_request();
	let user_messages = request.user_messages();
	// channel events surface to the webview; a dead channel cancels
	// generation instead of burning CPU on unseen output
	let sender = on_event;
	generate_and_respond(
		&state,
		&request,
		&user_messages,
		true,
		move |kind, content| {
			sender
				.send(StreamEvent::new(kind.as_str(), content))
				.is_ok()
		},
	)
	.await
}

#[tauri::command]
pub fn cancel_generation(state: State<'_, AppState>) {
	state
		.generation_cancel
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.store(true, Ordering::Relaxed);
}

/// Start capturing microphone audio (Rust-side, bypasses the webview).
/// Async so the CoreAudio device probing never blocks the main thread.
#[tauri::command]
pub async fn start_voice_capture() -> Result<(), String> {
	tauri::async_runtime::spawn_blocking(crate::voice::start_capture)
		.await
		.map_err(|e| e.to_string())?
}

/// Stop capturing and return the recording as a 16 kHz mono WAV (raw bytes).
/// Async: folding + resampling + WAV-encoding of a multi-minute recording
/// is real CPU work and must not run on the main thread.
#[tauri::command]
pub async fn stop_voice_capture() -> Result<tauri::ipc::Response, String> {
	let wav = tauri::async_runtime::spawn_blocking(crate::voice::stop_capture)
		.await
		.map_err(|e| e.to_string())??;
	Ok(tauri::ipc::Response::new(wav))
}

/// Test notification command, also used to trigger the daily reminder
/// manually from settings.
#[tauri::command]
pub fn send_test_notification(app: AppHandle) {
	let _ = app
		.notification()
		.builder()
		.title("Brainstory")
		.body("Time to think out loud!")
		.show();
}

fn local_llm(state: &State<'_, AppState>) -> Result<Arc<crate::llm::LocalLlm>, String> {
	state
		.runtime
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.llm
		.clone()
		.ok_or_else(|| {
			"Language model not downloaded yet. Open Settings > AI Models to download one."
				.to_string()
		})
}

fn external_llm(settings: &AiSettings) -> Result<ExternalLlm, String> {
	if settings.ext_llm_base_url.is_empty() {
		return Err("external LLM endpoint is not configured".into());
	}
	ExternalLlm::new(
		&settings.ext_llm_base_url,
		&settings.ext_llm_api_key,
		if settings.ext_llm_model.is_empty() {
			"default"
		} else {
			&settings.ext_llm_model
		},
	)
}

/// Run one LLM call on the configured backend (local llama.cpp or external
/// OpenAI-compatible endpoint), streaming pieces through `on_chunk`.
/// Returns the final text plus the prompt token count (0 when unknown, as
/// with external endpoints).
async fn run_generation(
	state: &State<'_, AppState>,
	settings: &AiSettings,
	system: String,
	messages: &[ChatMessage],
	summarize: bool,
	cancel: Arc<AtomicBool>,
	on_chunk: impl FnMut(String) + Send + 'static,
) -> Result<(String, usize), String> {
	let max_tokens = if summarize {
		crate::llm::MAX_NEW_TOKENS_RESULT
	} else {
		crate::llm::MAX_NEW_TOKENS_RESPONSE
	};
	if settings.uses_external_llm() {
		let client = external_llm(settings)?;
		let (text, _prompt_tokens) = client
			.generate(&system, messages, &cancel, max_tokens, on_chunk)
			.await?;
		Ok((text, 0))
	} else {
		let engine = local_llm(state)?;
		let messages = messages.to_vec();
		tauri::async_runtime::spawn_blocking(move || {
			engine.generate(&system, &messages, summarize, &cancel, on_chunk)
		})
		.await
		.map_err(|e| e.to_string())?
	}
}

/// Pull the first JSON object or array out of an LLM response.
fn extract_json(text: &str) -> Option<serde_json::Value> {
	let trimmed = text.trim();
	if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
		return Some(v);
	}
	let start = text.find(['{', '['])?;
	let end = text.rfind(['}', ']'])?;
	if end <= start {
		return None;
	}
	serde_json::from_str(&text[start..=end]).ok()
}

/// extract_json plus the feedback-document contract: the value must be an
/// object whose feedback_items (when present) is an array. Anything else
/// is treated as absent rather than stored broken.
fn extract_feedback_json(text: &str) -> Option<serde_json::Value> {
	let value = extract_json(text)?;
	let items = value.get("feedback_items");
	match items {
		None => {
			log::warn!("structured feedback JSON has no feedback_items key");
			None
		}
		Some(items) if items.is_array() => Some(value),
		Some(_) => {
			log::warn!("structured feedback JSON has a non-array feedback_items");
			None
		}
	}
}

#[cfg(test)]
mod tests {
	use super::extract_json;

	#[test]
	fn parses_pure_json() {
		let v = extract_json(r#"{"a": 1}"#).expect("pure object");
		assert_eq!(v["a"], 1);
	}

	#[test]
	fn extracts_object_from_prose() {
		let v = extract_json("Here is your document:\n```json\n{\"title\": \"T\"}\n```")
			.expect("embedded object");
		assert_eq!(v["title"], "T");
	}

	#[test]
	fn extracts_array_from_prose() {
		let v = extract_json("prefix [1, 2, 3] suffix").expect("embedded array");
		assert_eq!(v, serde_json::json!([1, 2, 3]));
	}

	#[test]
	fn rejects_nested_mismatched_brackets() {
		// first '{' before any '[', last '}' - unparseable slice yields None
		assert!(extract_json("no json here").is_none());
	}

	#[test]
	fn rejects_broken_json() {
		assert!(extract_json("{not json}").is_none());
	}

	#[test]
	fn multiple_objects_do_not_panic() {
		// first '{' to last '}' spans two objects: the slice is not valid
		// JSON, so the contract is a clean None, never a panic
		assert!(extract_json("a {\"x\": 1} b {\"y\": 2}").is_none());
	}
}

#[cfg(test)]
mod feedback_json_tests {
	use super::extract_feedback_json;

	#[test]
	fn accepts_wellformed_feedback_documents() {
		let v = extract_feedback_json(r#"{"feedback_items": [{"oid_heading_text": "1## A"}]}"#)
			.expect("valid");
		assert!(v["feedback_items"].is_array());
	}

	#[test]
	fn empty_feedback_items_is_valid() {
		assert!(extract_feedback_json(r#"{"feedback_items": []}"#).is_some());
	}

	#[test]
	fn rejects_missing_or_non_array_feedback_items() {
		assert!(extract_feedback_json(r#"{"something": "else"}"#).is_none());
		assert!(extract_feedback_json(r#"{"feedback_items": "nope"}"#).is_none());
		assert!(extract_feedback_json("no json at all").is_none());
	}
}
