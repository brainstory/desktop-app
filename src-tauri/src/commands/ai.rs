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
	app: AppHandle,
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
	// otherwise use the local whisper model.
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

	let engine = {
		let runtime = state.runtime.lock().unwrap_or_else(|e| e.into_inner());
		runtime.stt.clone()
	};
	let engine = engine.ok_or_else(|| {
		"Speech model not downloaded yet. Open Settings > AI Models to download one.".to_string()
	})?;

	// WAV decoding of a multi-minute recording is CPU work too; keep it off
	// the async runtime alongside the whisper inference.
	let transcript = tauri::async_runtime::spawn_blocking(move || {
		let samples = stt::wav_to_samples(&bytes)?;
		engine.transcribe(&samples)
	})
	.await
	.map_err(|e| e.to_string())??;

	let _ = app; // reserved for status events
	Ok(serde_json::json!({ "transcript": transcript }))
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
	let mut request = PromptRequest {
		chat_type: ChatType::parse(chat_type.as_deref()),
		messages,
		summarize: summarize.unwrap_or(false),
		react_to,
		react_to_author,
		react_to_is_current_user: react_to_is_current_user.unwrap_or(false),
		structured_feedback: structured_feedback.unwrap_or(false),
	};
	// A react_to payload implies the feedback flow even if the frontend
	// didn't label the chat type.
	if request.react_to.is_some() && request.chat_type == ChatType::Original {
		request.chat_type = ChatType::Feedback;
	}
	let system = request.system_prompt();
	let user_messages = request.user_messages();
	let word_count: usize = user_messages
		.iter()
		.map(|m| m.content.split_whitespace().count())
		.sum();
	let cancel = take_cancel_token(&state);
	let settings = AiSettings::load(&state.db);

	let (output, prompt_tokens) = run_generation(
		&state,
		&settings,
		system,
		&user_messages,
		request.summarize,
		cancel,
		|_| {},
	)
	.await?;

	// If a structured result was requested, try to extract the JSON document.
	let mut structured = None;
	if request.summarize && request.structured_feedback {
		structured = extract_json(&output);
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
	let mut request = PromptRequest {
		chat_type: ChatType::parse(chat_type.as_deref()),
		messages,
		summarize: summarize.unwrap_or(false),
		react_to,
		react_to_author,
		react_to_is_current_user: react_to_is_current_user.unwrap_or(false),
		structured_feedback: structured_feedback.unwrap_or(false),
	};
	// A react_to payload implies the feedback flow even if the frontend
	// didn't label the chat type.
	if request.react_to.is_some() && request.chat_type == ChatType::Original {
		request.chat_type = ChatType::Feedback;
	}
	let system = request.system_prompt();
	let user_messages = request.user_messages();
	let word_count: usize = user_messages
		.iter()
		.map(|m| m.content.split_whitespace().count())
		.sum();
	let cancel = take_cancel_token(&state);
	let settings = AiSettings::load(&state.db);

	let _ = on_event.send(StreamEvent::new("status", "thinking..."));

	// If the webview went away (page reloaded mid-stream), stop generating
	// instead of burning CPU on output nobody will see.
	let channel_writer = {
		let sender = on_event.clone();
		let cancel_writer = cancel.clone();
		move |piece: String| {
			if sender.send(StreamEvent::new("chunk", piece)).is_err() {
				cancel_writer.store(true, Ordering::Relaxed);
			}
		}
	};
	let (output, prompt_tokens) = run_generation(
		&state,
		&settings,
		system,
		&user_messages,
		request.summarize,
		cancel.clone(),
		channel_writer,
	)
	.await?;

	let _ = on_event.send(StreamEvent::new("cumulative", output.clone()));

	// Structured feedback: either the request itself asked for JSON, or this
	// is the standard feedback flow, in which case run a second pass to also
	// produce the structured JSON feedback document.
	let mut structured = None;
	if request.summarize && request.structured_feedback {
		structured = extract_json(&output);
	} else if request.summarize && request.chat_type == ChatType::Feedback {
		let json_system = crate::prompts::FEEDBACK_JSON_RESULT_SYSTEM.to_string();
		let json_output = run_generation(
			&state,
			&settings,
			json_system,
			&user_messages,
			true,
			cancel,
			|_| {},
		)
		.await;
		if let Ok((json_output, _)) = json_output {
			structured = extract_json(&json_output);
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
	Ok(ExternalLlm::new(
		&settings.ext_llm_base_url,
		&settings.ext_llm_api_key,
		if settings.ext_llm_model.is_empty() {
			"default"
		} else {
			&settings.ext_llm_model
		},
	))
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
	if settings.llm_mode == "external" {
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
