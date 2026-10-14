use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Manager, State};
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

/// Cancel any in-flight transcription and return the fresh token for this
/// one (the same swap discipline as `take_cancel_token`).
fn take_transcription_token(state: &AppState) -> Arc<AtomicBool> {
	let token = Arc::new(AtomicBool::new(false));
	let mut current = state
		.transcription_cancel
		.lock()
		.unwrap_or_else(|e| e.into_inner());
	let old = std::mem::replace(&mut *current, token.clone());
	old.store(true, Ordering::Relaxed);
	token
}

/// RAII counter for in-flight generations: a model switch (and the
/// quick-capture shortcut) waits for these to drain instead of yanking
/// the page or the engine's memory out from under a live generation.
struct GenerationGuard<'a>(&'a AppState);

impl Drop for GenerationGuard<'_> {
	fn drop(&mut self) {
		self.0.active_generations.fetch_sub(1, Ordering::SeqCst);
	}
}

fn enter_generation(state: &AppState) -> GenerationGuard<'_> {
	state.active_generations.fetch_add(1, Ordering::SeqCst);
	GenerationGuard(state)
}

/// Cancel an in-flight transcription. The token is checked before the
/// request and every 150 ms while it is in flight. The local whisper (and
/// Apple Speech) inference itself cannot be interrupted: a cancelled one
/// keeps running to completion in the background and its result is
/// discarded.
#[tauri::command]
pub fn cancel_transcription(app: AppHandle) {
	let state = app.state::<AppState>();
	let token = state
		.transcription_cancel
		.lock()
		.unwrap_or_else(|e| e.into_inner());
	token.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub async fn transcribe(
	app: AppHandle,
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

	// keychain/DB reads are blocking: keep them off the async workers
	let settings = {
		let app = app.clone();
		tauri::async_runtime::spawn_blocking(move || app.state::<AppState>().ai_settings())
			.await
			.map_err(|e| format!("read ai settings task failed: {e}"))?
	};
	// This transcription's cancel token: a previous one is cancelled by
	// the swap, and `cancel_transcription` reaches this one.
	let cancel = take_transcription_token(&app.state::<AppState>());
	// STT offload rule: the external endpoint when switched on (sttMode);
	// otherwise the engine setting picks Apple Speech or local whisper.
	if settings.uses_external_stt() {
		let transcript = stt::transcribe_external(
			&settings.ext_stt_base_url,
			&settings.ext_stt_api_key,
			&settings.ext_stt_model,
			bytes,
			&cancel,
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
		if cancel.load(Ordering::Relaxed) {
			return Err(stt::TRANSCRIPTION_CANCELLED.into());
		}
		let apple_bytes = bytes.clone();
		let locale = settings.stt_language.clone();
		let result = tauri::async_runtime::spawn_blocking(move || {
			crate::stt_apple::transcribe(&apple_bytes, &locale)
		})
		.await
		.map_err(|e| e.to_string());
		// a cancel while Apple was transcribing must not fall through to
		// whisper (nor report a success)
		if cancel.load(Ordering::Relaxed) {
			return Err(stt::TRANSCRIPTION_CANCELLED.into());
		}
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

	// the whisper slot may still be filling (startup load or a model
	// switch): wait instead of reporting "not downloaded"
	let engine = await_stt_engine(app.state::<AppState>().inner()).await?;
	if cancel.load(Ordering::Relaxed) {
		return Err(stt::TRANSCRIPTION_CANCELLED.into());
	}

	// WAV decoding of a multi-minute recording is CPU work too; keep it off
	// the async runtime alongside the whisper inference. The configured
	// language rides along so multilingual models honor it.
	let language = settings.stt_language.clone();
	let outcome = tauri::async_runtime::spawn_blocking(move || {
		let samples = stt::wav_to_samples(&bytes)?;
		// near-digital-silence (muted mic, bumped cable): whisper would
		// only hallucinate over it; report nothing was heard
		if stt::is_silent(&samples) {
			return Ok(String::new());
		}
		engine.transcribe(&samples, &language)
	})
	.await
	.map_err(|e| e.to_string());
	// whisper cannot be interrupted mid-inference: a cancel discards the
	// outcome (the background call finishes on its own) and reports the
	// same cancellation the UI already knows
	if cancel.load(Ordering::Relaxed) {
		return Err(stt::TRANSCRIPTION_CANCELLED.into());
	}
	let transcript = outcome??;

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
	// second_pass: run the structured-JSON second pass (streaming only)
	second_pass: bool,
	// on_event returns false when the channel is dead: cancel generation
	on_event: F,
) -> Result<serde_json::Value, String>
where
	F: Fn(EmitKind, &str) -> bool + Send + Sync + Clone + 'static,
{
	let system = request.system_prompt();
	let word_count: usize = request
		.messages
		.iter()
		.map(|m| m.content.split_whitespace().count())
		.sum();
	let cancel = take_cancel_token(state);
	// A warm-cache clone in practice (the loader seeds the cache at
	// startup); a cold read only happens after a failed save dropped it,
	// so this stays synchronous instead of threading the app handle
	// through the whole generation path.
	let settings = state.ai_settings();

	on_event(EmitKind::Status, "thinking...");

	// If the webview went away (page reloaded mid-stream), stop generating
	// instead of burning CPU on output nobody will see.
	let cancel_for_writer = cancel.clone();
	let emit = on_event.clone();
	let status_emit = on_event.clone();
	let on_status = move |text: &str| {
		status_emit(EmitKind::Status, text);
	};
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
	// Summary generations serialize the conversation inside the engine
	// (the turns are fitted to its context first); chat generations take
	// the turns as-is.
	let render = request.summary_renderer();
	let (mut output, prompt_tokens, dropped_turns) = run_generation(
		state,
		&settings,
		system,
		&request.messages,
		request.summarize,
		render.clone(),
		cancel.clone(),
		channel_writer,
		&on_status,
	)
	.await?;

	// A chat reply is one or two sentences plus ONE question; a runaway
	// model writing several questions buries the conversation point.
	if !request.summarize {
		let trimmed = trim_after_first_question(&output);
		if trimmed != output {
			log::warn!("chat reply carried multiple question lines; trimmed to the first");
			output = trimmed;
		}
	}

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
		let json_system = crate::prompts::Prompt::FeedbackJsonResult
			.text()
			.to_string();
		let json_output = run_generation(
			state,
			&settings,
			json_system,
			&request.messages,
			true,
			render.clone(),
			cancel,
			|_| {},
			&on_status,
		)
		.await;
		match json_output {
			Ok((json_output, _, _)) => structured = extract_feedback_json(&json_output),
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
		// how many transcript turns the summary fit had to leave out
		"truncated": dropped_turns > 0,
		"dropped_turns": dropped_turns,
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
	state.refuse_if_shutting_down()?;
	generate_and_respond(&state, &request, false, |_, _| true).await
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
	state.refuse_if_shutting_down()?;
	// channel events surface to the webview; a dead channel cancels
	// generation instead of burning CPU on unseen output
	let sender = on_event;
	generate_and_respond(&state, &request, true, move |kind, content| {
		sender
			.send(StreamEvent::new(kind.as_str(), content))
			.is_ok()
	})
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

/// How long an AI command waits for an in-flight engine load (startup
/// or a model switch) before giving up, and how often it polls.
const LOAD_WAIT_POLL: std::time::Duration = std::time::Duration::from_millis(200);
const LOAD_WAIT_MAX: std::time::Duration = std::time::Duration::from_secs(60);

/// Resolve an engine slot that may still be filling: startup and model
/// switches load outside the AI commands, so an empty slot during a
/// load means "wait", not "not downloaded". `read_slot` returns the
/// current engine and its published status; `on_status` fires once
/// when the wait starts; an `Error` status maps through `failed`;
/// anything else with an empty slot keeps the not-downloaded message.
async fn await_engine_slot<T>(
	mut read_slot: impl FnMut() -> (Option<Arc<T>>, crate::models::EngineStatus),
	loading_msg: &'static str,
	on_status: Option<&(dyn Fn(&str) + Sync)>,
	failed: impl Fn(&crate::models::EngineStatus) -> String,
	not_downloaded: &'static str,
) -> Result<Arc<T>, String> {
	let mut announced = false;
	let started = std::time::Instant::now();
	loop {
		let (engine, status) = read_slot();
		if let Some(engine) = engine {
			return Ok(engine);
		}
		match status.state {
			crate::models::EngineState::Loading => {
				if !announced {
					if let Some(emit) = on_status {
						emit(loading_msg);
					}
					announced = true;
				}
				if started.elapsed() >= LOAD_WAIT_MAX {
					return Err("The model is still loading - try again in a moment.".into());
				}
				tokio::time::sleep(LOAD_WAIT_POLL).await;
			}
			crate::models::EngineState::Error => return Err(failed(&status)),
			_ => return Err(not_downloaded.into()),
		}
	}
}

/// The LLM slot, waiting out an in-flight load. A failed load surfaces
/// its error text instead of the misleading "not downloaded".
async fn await_local_llm(
	state: &AppState,
	on_status: &(dyn Fn(&str) + Sync),
) -> Result<Arc<crate::llm::LocalLlm>, String> {
	await_engine_slot(
		|| {
			let engine = state
				.runtime
				.lock()
				.unwrap_or_else(|e| e.into_inner())
				.llm
				.clone();
			let status = state
				.llm_status
				.lock()
				.unwrap_or_else(|e| e.into_inner())
				.clone();
			(engine, status)
		},
		"Loading the language model...",
		Some(on_status),
		|status| {
			format!(
				"Language model failed to load{}",
				status
					.error
					.as_deref()
					.map(|e| format!(": {e}"))
					.unwrap_or_default()
			)
		},
		"Language model not downloaded yet. Open Settings > AI Models to download one.",
	)
	.await
}

/// The whisper slot, waiting out an in-flight load. Only a whisper
/// load's error maps to its text: an error published for another
/// engine (e.g. the Apple Speech notice) with no whisper resident
/// means whisper was never downloaded, not that it failed.
async fn await_stt_engine(state: &AppState) -> Result<Arc<crate::stt::SttEngine>, String> {
	const NOT_DOWNLOADED: &str =
		"Speech model not downloaded yet. Open Settings > AI Models to download one.";
	await_engine_slot(
		|| {
			let engine = state
				.runtime
				.lock()
				.unwrap_or_else(|e| e.into_inner())
				.stt
				.clone();
			let status = state
				.stt_status
				.lock()
				.unwrap_or_else(|e| e.into_inner())
				.clone();
			(engine, status)
		},
		"Loading the speech model...",
		None,
		|status| match status.model_id.as_deref() {
			Some(id) if id.starts_with("whisper") => format!(
				"Speech model failed to load{}",
				status
					.error
					.as_deref()
					.map(|e| format!(": {e}"))
					.unwrap_or_default()
			),
			_ => NOT_DOWNLOADED.to_string(),
		},
		NOT_DOWNLOADED,
	)
	.await
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
/// Returns the final text, the prompt token count (0 when unknown, as
/// with external endpoints) and how many transcript turns a summary fit
/// dropped. `render` (Some = summary generation) serializes the fitted
/// turns into the single summary message inside the engine.
#[allow(clippy::too_many_arguments)]
async fn run_generation(
	state: &State<'_, AppState>,
	settings: &AiSettings,
	system: String,
	messages: &[ChatMessage],
	summarize: bool,
	render: Option<crate::llm::SummaryRender>,
	cancel: Arc<AtomicBool>,
	on_chunk: impl FnMut(String) + Send + 'static,
	on_status: &(dyn Fn(&str) + Sync),
) -> Result<(String, usize, usize), String> {
	let _generating = enter_generation(state.inner());
	let max_tokens = if summarize {
		crate::llm::MAX_NEW_TOKENS_RESULT
	} else {
		crate::llm::MAX_NEW_TOKENS_RESPONSE
	};
	if settings.uses_external_llm() {
		let client = external_llm(settings)?;
		let (text, _prompt_tokens, dropped_turns) = client
			.generate(
				&system,
				messages,
				render.as_deref(),
				&cancel,
				max_tokens,
				on_chunk,
			)
			.await?;
		Ok((text, 0, dropped_turns))
	} else {
		// the slot may still be filling (startup load or a model
		// switch): wait instead of reporting "not downloaded"
		let engine = await_local_llm(state.inner(), on_status).await?;
		let messages = messages.to_vec();
		tauri::async_runtime::spawn_blocking(move || {
			engine.generate(&system, &messages, render.as_deref(), &cancel, on_chunk)
		})
		.await
		.map_err(|e| e.to_string())?
	}
}

/// Keep a chat reply up to and including the first line that ends in a
/// question mark, but only when it asked more than one question: the
/// interview prompt wants a sentence or two plus one question, and
/// several stacked questions degrade the following turns.
fn trim_after_first_question(reply: &str) -> String {
	let lines: Vec<&str> = reply.lines().collect();
	let question_lines: Vec<usize> = lines
		.iter()
		.enumerate()
		.filter(|(_, line)| line.trim_end().ends_with('?'))
		.map(|(idx, _)| idx)
		.collect();
	match question_lines.first() {
		Some(&first) if question_lines.len() > 1 => lines[..=first].join("\n"),
		_ => reply.to_string(),
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
/// object with a feedback_items key holding an array. Output that is not
/// JSON, lacks the key, or holds a non-array is logged and treated as
/// absent (structured_result becomes null) rather than stored broken.
/// Unusable items (no feedback text, a non-string heading) and exact
/// repeats of an earlier item are dropped: each item becomes a comment
/// bubble on the idea.
fn extract_feedback_json(text: &str) -> Option<serde_json::Value> {
	let Some(mut value) = extract_json(text) else {
		// length only: the output is the user's own feedback text
		log::warn!(
			"structured feedback output is not valid JSON ({} chars); structured_result will be null",
			text.chars().count()
		);
		return None;
	};
	let items = value.get("feedback_items");
	match items {
		None => {
			log::warn!("structured feedback JSON has no feedback_items key");
			None
		}
		Some(items) if items.is_array() => {
			if let Some(items) = value["feedback_items"].as_array_mut() {
				let mut seen = std::collections::HashSet::new();
				items.retain(|item| {
					// same member contract as the share import: non-empty
					// string feedback_text and a string-or-absent heading
					// (a non-string heading can never be placed on a
					// section and would crash the heading parser later)
					super::share::is_usable_feedback_item(item)
						&& seen.insert((
							item["oid_heading_text"]
								.as_str()
								.unwrap_or_default()
								.to_owned(),
							item["feedback_text"]
								.as_str()
								.unwrap_or_default()
								.trim()
								.to_owned(),
						))
				});
			}
			Some(value)
		}
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
mod reply_trim_tests {
	use super::trim_after_first_question;

	#[test]
	fn a_single_question_reply_passes_through_unchanged() {
		let one = "That sounds fun.\nWhat is the first step you would take?";
		assert_eq!(trim_after_first_question(one), one);
		// trailing non-question lines after ONE question stay too
		let with_tail = "Great.\nWhat is the goal?\nTake your time.";
		assert_eq!(trim_after_first_question(with_tail), with_tail);
		// a reply with no questions at all is untouched
		let none = "Okay, noted. I will remember that.";
		assert_eq!(trim_after_first_question(none), none);
	}

	#[test]
	fn multiple_questions_are_trimmed_to_the_first() {
		let stacked = "Interesting!\nWhat happened next?\nWhy did that matter?\nAnd then?";
		assert_eq!(
			trim_after_first_question(stacked),
			"Interesting!\nWhat happened next?"
		);
		// the question mark must END its line; mid-line ones do not count
		let inline = "What? really.\nAre you sure about that plan?";
		assert_eq!(trim_after_first_question(inline), inline);
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
	fn feedback_items_do_not_need_labels() {
		// the LLM no longer picks emoji labels (people react instead);
		// documents without them are valid and older ones that still
		// carry labels keep loading
		let item = r#"{"oid_heading_text": "1## A", "matched_spans": [], "feedback_text": "f", "confidence": "high", "priority": "medium", "relationships": []}"#;
		let v = extract_feedback_json(&format!(r#"{{"feedback_items": [{item}]}}"#))
			.expect("no labels");
		assert!(v["feedback_items"][0].get("labels").is_none());
		let legacy = r#"{"feedback_items": [{"oid_heading_text": "1## A", "labels": [{"name": "agree", "emoji": "👍"}]}]}"#;
		assert!(extract_feedback_json(legacy).is_some());
	}

	#[test]
	fn drops_items_without_text_and_exact_repeats() {
		// E2B added an empty-text item for every section H never mentioned
		let v = extract_feedback_json(
			r#"{"feedback_items": [
				{"oid_heading_text": "1## A", "matched_spans": [], "feedback_text": "more ideas"},
				{"oid_heading_text": "2## B", "matched_spans": [], "feedback_text": ""},
				{"oid_heading_text": "3## C", "matched_spans": [], "feedback_text": "  "},
				{"oid_heading_text": "1## A", "matched_spans": ["x"], "feedback_text": "more ideas"},
				{"oid_heading_text": "3## C", "matched_spans": [], "feedback_text": "more ideas"}
			]}"#,
		)
		.expect("valid");
		let kept: Vec<(&str, &str)> = v["feedback_items"]
			.as_array()
			.unwrap()
			.iter()
			.map(|i| {
				(
					i["oid_heading_text"].as_str().unwrap(),
					i["feedback_text"].as_str().unwrap(),
				)
			})
			.collect();
		assert_eq!(kept, [("1## A", "more ideas"), ("3## C", "more ideas")]);
	}

	#[test]
	fn empty_feedback_items_is_valid() {
		assert!(extract_feedback_json(r#"{"feedback_items": []}"#).is_some());
	}

	/// Records every log line so a test can assert a failure was not
	/// silent. log allows one global logger per process; no other test
	/// installs one, and lines from concurrent tests are harmless here.
	struct CaptureLog(std::sync::Mutex<Vec<String>>);

	impl log::Log for CaptureLog {
		fn enabled(&self, _: &log::Metadata) -> bool {
			true
		}
		fn log(&self, record: &log::Record) {
			self.0
				.lock()
				.unwrap_or_else(|e| e.into_inner())
				.push(format!("{} {}", record.level(), record.args()));
		}
		fn flush(&self) {}
	}

	static CAPTURE: CaptureLog = CaptureLog(std::sync::Mutex::new(Vec::new()));

	#[test]
	fn unparseable_output_is_logged_not_silent() {
		let _ = log::set_logger(&CAPTURE);
		log::set_max_level(log::LevelFilter::Trace);
		// a distinctive length so the line can't come from another test
		let broken = "the model forgot the JSON entirely :(";
		let expected = format!("not valid JSON ({} chars)", broken.chars().count());
		assert!(extract_feedback_json(broken).is_none());
		let lines = CAPTURE.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
		assert!(
			lines
				.iter()
				.any(|l| l.starts_with("WARN") && l.contains(&expected)),
			"parse failure was not logged: {lines:?}"
		);
	}

	#[test]
	fn rejects_missing_or_non_array_feedback_items() {
		assert!(extract_feedback_json(r#"{"something": "else"}"#).is_none());
		assert!(extract_feedback_json(r#"{"feedback_items": "nope"}"#).is_none());
		assert!(extract_feedback_json("no json at all").is_none());
	}

	/// Members whose oid_heading_text is not a string (number/object)
	/// can never be placed on a section and crash the frontend heading
	/// parser at display time: they are dropped here, at the generation
	/// boundary, before the document is ever stored. Absent or null
	/// headings stay (aggregation logs and skips those comments).
	#[test]
	fn non_string_heading_members_are_dropped() {
		let v = extract_feedback_json(
			r#"{"feedback_items": [
				{"oid_heading_text": 1, "matched_spans": [], "feedback_text": "one"},
				{"oid_heading_text": {"n": 2}, "feedback_text": "two"},
				{"oid_heading_text": null, "matched_spans": [], "feedback_text": "null heading stays"},
				{"matched_spans": [], "feedback_text": "absent heading stays"},
				{"oid_heading_text": "2## B", "matched_spans": [], "feedback_text": "kept"}
			]}"#,
		)
		.expect("valid document");
		let kept: Vec<&str> = v["feedback_items"]
			.as_array()
			.unwrap()
			.iter()
			.map(|i| i["feedback_text"].as_str().unwrap())
			.collect();
		assert_eq!(
			kept,
			["null heading stays", "absent heading stays", "kept"],
			"non-string headings dropped, everything else kept"
		);
	}
}

#[cfg(test)]
mod engine_wait_tests {
	use super::{await_engine_slot, LOAD_WAIT_MAX, LOAD_WAIT_POLL};
	use crate::models::EngineStatus;
	use std::sync::{Arc, Mutex};
	use std::time::Duration;

	/// The wait must stay far below the 60 s cap in tests: a bug that
	/// polls past the cap would hang the suite for a minute per call.
	#[test]
	fn poll_and_cap_are_sane() {
		assert!(LOAD_WAIT_POLL >= Duration::from_millis(50));
		assert!(LOAD_WAIT_MAX >= Duration::from_secs(30));
	}

	#[tokio::test]
	async fn a_loading_slot_is_waited_out_until_the_engine_appears() {
		#[derive(Debug)]
		struct FakeEngine;
		let slot: Arc<Mutex<Option<Arc<FakeEngine>>>> = Arc::new(Mutex::new(None));
		let status: Arc<Mutex<EngineStatus>> =
			Arc::new(Mutex::new(EngineStatus::loading("gemma-4-E2B")));

		// the load completes 300 ms in, from another "thread"
		let filler_slot = Arc::clone(&slot);
		let filler_status = Arc::clone(&status);
		tokio::spawn(async move {
			tokio::time::sleep(Duration::from_millis(300)).await;
			*filler_slot.lock().unwrap() = Some(Arc::new(FakeEngine));
			*filler_status.lock().unwrap() = EngineStatus::ready(Some("gemma-4-E2B"));
		});

		let read_slot = move || (slot.lock().unwrap().clone(), status.lock().unwrap().clone());
		let announced = Arc::new(Mutex::new(Vec::<String>::new()));
		let seen = Arc::clone(&announced);
		let on_status = move |text: &str| seen.lock().unwrap().push(text.to_string());

		let engine = await_engine_slot(
			read_slot,
			"Loading the language model...",
			Some(&on_status),
			|_| "failed".to_string(),
			"not downloaded",
		)
		.await
		.expect("the engine appears once the load lands");

		assert!(
			Arc::strong_count(&engine) >= 1,
			"the engine resolved from the filled slot"
		);
		assert_eq!(
			*announced.lock().unwrap(),
			vec!["Loading the language model...".to_string()],
			"the loading status is announced exactly once"
		);
	}

	#[tokio::test]
	async fn a_missing_slot_keeps_the_not_downloaded_message_without_waiting() {
		#[derive(Debug)]
		struct FakeEngine;
		let slot: Arc<Mutex<Option<Arc<FakeEngine>>>> = Arc::new(Mutex::new(None));
		let status: Arc<Mutex<EngineStatus>> = Arc::new(Mutex::new(EngineStatus::missing()));

		let started = std::time::Instant::now();
		let read_slot = move || (slot.lock().unwrap().clone(), status.lock().unwrap().clone());
		let err = await_engine_slot(
			read_slot,
			"Loading the language model...",
			None,
			|_| "failed".to_string(),
			"Language model not downloaded yet. Open Settings > AI Models to download one.",
		)
		.await
		.expect_err("missing stays missing");
		assert_eq!(
			err,
			"Language model not downloaded yet. Open Settings > AI Models to download one."
		);
		assert!(
			started.elapsed() < Duration::from_secs(1),
			"a missing slot must not wait out the load timeout"
		);
	}

	#[tokio::test]
	async fn an_error_slot_surfaces_its_error_text() {
		#[derive(Debug)]
		struct FakeEngine;
		let slot: Arc<Mutex<Option<Arc<FakeEngine>>>> = Arc::new(Mutex::new(None));
		let status: Arc<Mutex<EngineStatus>> = Arc::new(Mutex::new(EngineStatus::error(
			Some("gemma-4-E4B"),
			"bad gguf header",
		)));

		let read_slot = move || (slot.lock().unwrap().clone(), status.lock().unwrap().clone());
		let err = await_engine_slot(
			read_slot,
			"Loading the language model...",
			None,
			|status| {
				format!(
					"Language model failed to load: {}",
					status.error.clone().unwrap_or_default()
				)
			},
			"not downloaded",
		)
		.await
		.expect_err("a failed load is an error, not a wait");
		assert_eq!(err, "Language model failed to load: bad gguf header");
	}

	/// The STT error mapping: only a whisper-model error maps to its
	/// text; an error published for another engine (the Apple Speech
	/// notice) with no whisper resident means whisper was never
	/// downloaded.
	#[tokio::test]
	async fn a_non_whisper_error_keeps_the_not_downloaded_message() {
		#[derive(Debug)]
		struct FakeEngine;
		let slot: Arc<Mutex<Option<Arc<FakeEngine>>>> = Arc::new(Mutex::new(None));
		let status: Arc<Mutex<EngineStatus>> = Arc::new(Mutex::new(EngineStatus::error(
			Some("apple-speech"),
			"Apple Speech requires macOS 26+ - using whisper instead",
		)));

		let read_slot = move || (slot.lock().unwrap().clone(), status.lock().unwrap().clone());
		let err = await_engine_slot(
			read_slot,
			"Loading the speech model...",
			None,
			|status| match status.model_id.as_deref() {
				Some(id) if id.starts_with("whisper") => "whisper failed".to_string(),
				_ => "Speech model not downloaded yet.".to_string(),
			},
			"Speech model not downloaded yet.",
		)
		.await
		.expect_err("no whisper resident");
		assert_eq!(err, "Speech model not downloaded yet.");
	}
}
