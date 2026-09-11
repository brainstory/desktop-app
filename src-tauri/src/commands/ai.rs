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
fn take_cancel_token(state: &AppState) -> Arc<AtomicBool> {
	let token = Arc::new(AtomicBool::new(false));
	state.generation_cancel.lock().unwrap().store(true, Ordering::Relaxed);
	*state.generation_cancel.lock().unwrap() = token.clone();
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

	let settings = AiSettings::load(&state.db);
	// STT offload rule: if an external STT endpoint is configured, use it;
	// otherwise use the local whisper model.
	if !settings.ext_stt_base_url.is_empty() {
		let transcript =
			stt::transcribe_external(&settings.ext_stt_base_url, &settings.ext_stt_api_key, &settings.ext_stt_model, bytes)
				.await?;
		return Ok(serde_json::json!({ "transcript": transcript }));
	}

	let samples = stt::wav_to_samples(&bytes)?;
	let engine = {
		let runtime = state.runtime.lock().unwrap();
		runtime.stt.clone()
	};
	let engine = engine.ok_or_else(|| {
		"Speech model not downloaded yet. Open Settings > AI Models to download one.".to_string()
	})?;

	let transcript = tauri::async_runtime::spawn_blocking(move || engine.transcribe(&samples))
		.await
		.map_err(|e| e.to_string())??;

	let _ = app; // reserved for status events
	Ok(serde_json::json!({ "transcript": transcript }))
}

#[tauri::command]
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

	let output =
		run_generation(&state, &settings, system, &user_messages, request.summarize, cancel, |_| {})
			.await?;

	// If a structured result was requested, try to extract the JSON document.
	let mut structured = None;
	if request.summarize && request.structured_feedback {
		structured = extract_json(&output);
	}

	Ok(serde_json::json!({
		"response": output,
		"request_message_tokens": 0,
		"request_word_count": word_count,
		"structured_result": structured,
	}))
}

#[tauri::command]
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

	let channel_writer = {
		let sender = on_event.clone();
		move |piece: String| {
			let _ = sender.send(StreamEvent::new("chunk", piece));
		}
	};
	let output = run_generation(
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
		if let Ok(json_output) = json_output {
			structured = extract_json(&json_output);
		}
	}

	Ok(serde_json::json!({
		"response": output,
		"request_message_tokens": 0,
		"request_word_count": word_count,
		"structured_result": structured,
	}))
}

#[tauri::command]
pub fn cancel_generation(state: State<'_, AppState>) {
	state.generation_cancel.lock().unwrap().store(true, Ordering::Relaxed);
}

/// Start capturing microphone audio (Rust-side, bypasses the webview).
#[tauri::command]
pub fn start_voice_capture() -> Result<(), String> {
	crate::voice::start_capture()
}

/// Stop capturing and return the recording as a 16 kHz mono WAV (raw bytes).
#[tauri::command]
pub fn stop_voice_capture() -> Result<tauri::ipc::Response, String> {
	let wav = crate::voice::stop_capture()?;
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
		.unwrap()
		.llm
		.clone()
		.ok_or_else(|| {
			"Language model not downloaded yet. Open Settings > AI Models to download one.".to_string()
		})
}

fn external_llm(settings: &AiSettings) -> Result<ExternalLlm, String> {
	if settings.ext_llm_base_url.is_empty() {
		return Err("external LLM endpoint is not configured".into());
	}
	Ok(ExternalLlm::new(
		&settings.ext_llm_base_url,
		&settings.ext_llm_api_key,
		if settings.ext_llm_model.is_empty() { "default" } else { &settings.ext_llm_model },
	))
}

/// Run one LLM call on the configured backend (local llama.cpp or external
/// OpenAI-compatible endpoint), streaming pieces through `on_chunk`.
async fn run_generation(
	state: &State<'_, AppState>,
	settings: &AiSettings,
	system: String,
	messages: &[ChatMessage],
	summarize: bool,
	cancel: Arc<AtomicBool>,
	on_chunk: impl FnMut(String) + Send + 'static,
) -> Result<String, String> {
	if settings.llm_mode == "external" {
		let client = external_llm(settings)?;
		client.generate(&system, messages, &cancel, on_chunk).await
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
