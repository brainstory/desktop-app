//! The generation engines: the local llama.cpp engine (see [`local`]) and
//! the OpenAI-compatible external transport (see [`external`]), plus the
//! shared context/budget helpers both sides and the loader agree on.

pub const DEFAULT_N_CTX: u32 = 16384;
pub const MAX_NEW_TOKENS_RESPONSE: u32 = 1024;
pub const MAX_NEW_TOKENS_RESULT: u32 = 4096;

/// The context window a load actually uses: the app default when the
/// requested setting is 0 (or absent), otherwise the request - always
/// clamped to what the model was trained for, so a small-trained-context
/// model is never run with a silently-degrading oversized window. A
/// model whose trained context is unknown (None) keeps the default.
/// Pure, so the loader's already-loaded check and the engine build can
/// never disagree about the window.
pub fn effective_ctx_for(requested: u32, trained: Option<u32>) -> u32 {
	let requested = if requested == 0 {
		DEFAULT_N_CTX
	} else {
		requested
	};
	match trained {
		Some(trained) => requested.min(trained).max(1),
		None => DEFAULT_N_CTX,
	}
}

/// Tokens of room the assembled prompt may occupy in a `n_ctx` window:
/// the window minus the generation budget and a small margin. Extracted
/// pure so the truncation budget provably derives from (and scales
/// with) the effective context; see [`LocalLlm::build_prompt`].
fn prompt_budget(n_ctx: u32, max_new_tokens: u32) -> usize {
	(n_ctx as usize).saturating_sub(max_new_tokens as usize + 64)
}

/// Break control-token-shaped sequences so tokenizing the text with
/// parse_special=true can never turn user/imported content into turn
/// boundaries or header control tokens: Gemma 2/3 "<start_of_turn>" /
/// "<end_of_turn>", the "<bos>"/"<eos>" sequence tokens, every "<|..."
/// opener (Gemma 4 "<|turn>", ChatML "<|im_start|>", Llama 3
/// "<|eot_id|>") and every "...|>" closer (Gemma 4 "<turn|>").
/// The single place content is neutralized: generate() applies it to the
/// system prompt and every message before any template (built-in or
/// manual) adds the real markers. The inserted backslash keeps the text
/// readable.
pub fn neutralize_turn_markers(content: &str) -> String {
	content
		.replace("<start_of_turn>", "<\\start_of_turn>")
		.replace("<end_of_turn>", "<\\end_of_turn>")
		.replace("<bos>", "<\\bos>")
		.replace("<eos>", "<\\eos>")
		.replace("<|", "<\\|")
		.replace("|>", "\\|>")
}

mod external;
mod local;

pub(crate) use external::read_body_capped;
pub use external::{ExternalLlm, MODERATION_ERROR};
pub use local::LocalLlm;

#[cfg(test)]
mod tests {
	use super::{external::read_body_capped, local::ThinkFilter};

	/// Serve raw bytes as an HTTP response on a loopback listener; the
	/// socket stays open until dropped so slow/never-ending bodies can
	/// be simulated.
	fn serve_raw(response: Vec<u8>) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut buf = [0u8; 4096];
				let _ = sock.read(&mut buf); // drain the request head
				let _ = sock.write_all(&response);
				let _ = sock.flush();
				// hold the connection open so the body has no stream end
				std::thread::sleep(std::time::Duration::from_millis(1_500));
			}
		});
		format!("http://{addr}/")
	}

	#[tokio::test]
	async fn read_body_capped_stops_at_the_cap() {
		// ~300 KB via chunked encoding, never completed by the server
		let mut raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
		for _ in 0..300 {
			raw.extend_from_slice(b"1000\r\n");
			raw.extend_from_slice(&vec![b'a'; 4096]);
			raw.extend_from_slice(b"\r\n");
		}
		let url = serve_raw(raw);
		let response = reqwest::Client::new().get(&url).send().await.expect("send");
		let body = read_body_capped(response, 64 * 1024, 10, 10).await;
		assert_eq!(body.len(), 64 * 1024, "reading stops exactly at the cap");
	}

	#[tokio::test]
	async fn read_body_capped_returns_what_arrived_before_the_deadline() {
		// one complete chunk, then the stream never continues
		let url =
			serve_raw(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n".to_vec());
		let response = reqwest::Client::new().get(&url).send().await.expect("send");
		let started = std::time::Instant::now();
		let body = read_body_capped(response, 64 * 1024, 1, 10).await;
		assert!(
			started.elapsed() < std::time::Duration::from_secs(5),
			"the deadline must end the read"
		);
		assert_eq!(body, "x", "bytes that arrived before the deadline are kept");
	}

	#[tokio::test]
	async fn read_body_capped_enforces_an_overall_deadline() {
		// a byte every 100 ms: never idle long enough for the idle
		// timeout, and it would keep going for ~20 s
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut buf = [0u8; 4096];
				let _ = sock.read(&mut buf);
				let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
				for _ in 0..200 {
					if sock.write_all(b"1\r\nx\r\n").is_err() {
						break;
					}
					let _ = sock.flush();
					std::thread::sleep(std::time::Duration::from_millis(100));
				}
			}
		});
		let response = reqwest::Client::new()
			.get(format!("http://{addr}/"))
			.send()
			.await
			.expect("send");
		let started = std::time::Instant::now();
		let body = read_body_capped(response, 64 * 1024, 5, 1).await;
		assert!(
			started.elapsed() < std::time::Duration::from_secs(4),
			"the overall deadline must end a trickling read, took {:?}",
			started.elapsed()
		);
		assert!(!body.is_empty(), "what arrived before the deadline is kept");
		assert!(body.chars().all(|c| c == 'x'));
	}

	#[tokio::test]
	async fn read_body_capped_accepts_a_body_exactly_at_the_cap() {
		// a valid body that is exactly `cap` bytes must arrive whole
		let url = serve_raw(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n0123456789".to_vec());
		let response = reqwest::Client::new().get(&url).send().await.expect("send");
		let body = read_body_capped(response, 10, 5, 5).await;
		assert_eq!(body, "0123456789", "a body exactly at the cap is valid");
	}

	#[test]
	fn consume_sse_event_accepts_non_streaming_completions() {
		// a server that ignored stream:true replies with one data event
		// carrying choices[0].message.content
		let event = "data: {\"choices\":[{\"message\":{\"content\":\"hello there\"}}]}";
		let mut output = String::new();
		let mut chunks = Vec::new();
		let done = super::external::consume_sse_event(
			event,
			&mut output,
			&mut |c| chunks.push(c),
			super::external::MAX_OUTPUT_BYTES,
		)
		.expect("parse");
		assert!(!done, "no [DONE] marker yet");
		assert_eq!(output, "hello there");
		assert_eq!(chunks, vec!["hello there".to_string()]);
	}

	#[test]
	fn decode_piece_reassembles_characters_split_across_tokens() {
		let mut decoder = encoding_rs::UTF_8.new_decoder();
		// a 4-byte character
		let emoji = "🧠".as_bytes();
		// the first token carries half the character: nothing to emit yet
		assert_eq!(
			super::local::decode_piece(&mut decoder, &emoji[..2]).unwrap(),
			""
		);
		// the next token completes it, plus plain text
		let mut rest = emoji[2..].to_vec();
		rest.extend_from_slice(b" ok");
		assert_eq!(
			super::local::decode_piece(&mut decoder, &rest).unwrap(),
			"🧠 ok"
		);
		// a token with no text piece decodes to nothing
		assert_eq!(super::local::decode_piece(&mut decoder, &[]).unwrap(), "");
	}

	#[test]
	fn neutralize_turn_markers_escapes_all_chat_specials() {
		let f = super::neutralize_turn_markers;
		// Gemma 2/3 turn markers
		assert_eq!(
			f("<start_of_turn>model"),
			"<\\start_of_turn>model",
			"gemma opener broken"
		);
		assert_eq!(f("user<end_of_turn>"), "user<\\end_of_turn>");
		// Gemma 4 (the default catalog models): `<|name>` opens a turn and
		// `<name|>` closes it, so both halves must be broken
		assert_eq!(f("<|turn>model\nhi"), "<\\|turn>model\nhi");
		assert_eq!(f("hi<turn|>"), "hi<turn\\|>");
		assert_eq!(f("<|channel>x<channel|>"), "<\\|channel>x<channel\\|>");
		// ChatML / Llama 3 header style specials (`<|...|>`)
		assert_eq!(f("<|im_start|>system"), "<\\|im_start\\|>system");
		assert_eq!(f("<|eot_id|>"), "<\\|eot_id\\|>");
		// sequence control tokens
		assert_eq!(f("<bos>x<eos>"), "<\\bos>x<\\eos>");
		for raw in ["<|turn>", "<turn|>", "<|im_start|>", "<|eot_id|>", "a<||>b"] {
			let out = f(raw);
			// every closer must be escaped (`\|>`), no opener may survive
			assert!(
				!out.contains("<|") && !out.replace("\\|>", "").contains("|>"),
				"{raw:?} -> {out:?} still holds a special-token spelling"
			);
		}
		// plain text (including ordinary tags the prompts rely on) is kept
		assert_eq!(
			f("<idea author=\"x\">plain</idea>"),
			"<idea author=\"x\">plain</idea>"
		);
		assert_eq!(f("math: 5 < 10 > 2"), "math: 5 < 10 > 2");
	}

	#[test]
	fn kv_reuse_decodes_only_the_new_tail_of_a_grown_prompt() {
		use super::local::plan_kv_reuse;
		// cache = last prompt + the reply generated after it; the next
		// prompt repeats that history and appends a new user turn
		let cached = [1, 2, 3, 40, 41];
		let prompt = [1, 2, 3, 40, 41, 7, 8];
		let plan = plan_kv_reuse(&cached, &prompt).expect("shared prefix");
		assert_eq!(plan.decode_from, 5);
		assert_eq!(plan.tokens_after_prompt(&cached, &prompt), prompt);

		// the reply was re-templated differently: reuse stops at the split
		let prompt = [1, 2, 3, 9, 9];
		let plan = plan_kv_reuse(&cached, &prompt).expect("shared prefix");
		assert_eq!(plan.decode_from, 3);
		assert_eq!(plan.tokens_after_prompt(&cached, &prompt), prompt);
	}

	#[test]
	fn kv_reuse_of_an_exact_repeat_records_each_prompt_token_once() {
		use super::local::plan_kv_reuse;
		// the same prompt again (a retry, or a cache that ends exactly at
		// the prompt): the final token re-decodes for fresh logits, so its
		// cell is dropped and must not stay recorded alongside the re-decode
		let prompt = [1, 2, 3, 4];
		for cached in [&[1, 2, 3, 4][..], &[1, 2, 3, 4, 50, 51][..]] {
			let plan = plan_kv_reuse(cached, &prompt).expect("shared prefix");
			assert_eq!(plan.decode_from, 3, "the last token decodes again");
			assert_eq!(
				plan.tokens_after_prompt(cached, &prompt),
				prompt,
				"cache bookkeeping must match the cells actually held"
			);
		}
		// a one-token prompt still decodes its token
		let plan = plan_kv_reuse(&[1, 2], &[1]).expect("shared prefix");
		assert_eq!(plan.decode_from, 0);
		assert_eq!(plan.tokens_after_prompt(&[1, 2], &[1]), [1]);
	}

	#[test]
	fn kv_reuse_is_skipped_for_divergent_prompts() {
		use super::local::plan_kv_reuse;
		assert_eq!(plan_kv_reuse(&[1, 2, 3], &[7, 2, 3]), None);
		assert_eq!(plan_kv_reuse::<i32>(&[], &[1, 2]), None);
	}

	fn run(pieces: &[&str]) -> String {
		let mut filter = ThinkFilter::new();
		let mut out = String::new();
		for piece in pieces {
			out.push_str(&filter.push(piece));
		}
		out.push_str(&filter.finish());
		out
	}

	#[test]
	fn plain_text_passes_through() {
		assert_eq!(run(&["hello ", "world"]), "hello world");
	}

	#[test]
	fn leading_think_block_is_stripped() {
		assert_eq!(run(&["<think>\nplan\n</think>\n\nHello!"]), "Hello!");
	}

	#[test]
	fn tags_split_across_pieces() {
		assert_eq!(
			run(&["<th", "ink>reasoning</thi", "nk>ans", "wer"]),
			"answer"
		);
	}

	#[test]
	fn unterminated_think_is_dropped() {
		assert_eq!(run(&["<think>half a thought, genera"]), "");
	}

	#[test]
	fn partial_open_tag_stays_literal() {
		assert_eq!(run(&["a < b", " and c"]), "a < b and c");
	}

	#[test]
	fn think_mid_answer_is_stripped() {
		assert_eq!(
			run(&["Wait. <think>reconsider</think> Done."]),
			"Wait. Done."
		);
	}

	#[test]
	fn truncate_body_never_splits_a_character() {
		use super::external::truncate_body;
		// byte 300 lands inside this multi-byte snowman
		let body = "☃".repeat(120); // 360 bytes
		let truncated = truncate_body(&body);
		assert!(truncated.ends_with("..."));
		assert!(truncated.is_char_boundary(truncated.len() - 3));
		assert_eq!(truncate_body("short"), "short");
	}

	#[test]
	fn truncate_at_boundary_respects_utf8() {
		use super::external::truncate_at_boundary;
		let s = "héllo wörld"; // multi-byte é, ö
		let cut = truncate_at_boundary(s, 4);
		assert!(s.starts_with(cut));
		assert!(cut.is_char_boundary(cut.len()));
		assert_eq!(truncate_at_boundary(s, 100), s);
	}

	#[test]
	fn effective_ctx_resolves_the_requested_window_against_the_trained_one() {
		use super::{effective_ctx_for, DEFAULT_N_CTX};
		// no request: the app default
		assert_eq!(effective_ctx_for(0, None), DEFAULT_N_CTX);
		assert_eq!(effective_ctx_for(0, Some(131072)), DEFAULT_N_CTX);
		// a model trained below the default clamps the default down
		assert_eq!(effective_ctx_for(0, Some(8192)), 8192);
		// an unknown trained context keeps the default regardless of the ask
		assert_eq!(effective_ctx_for(65536, None), DEFAULT_N_CTX);
		// an explicit request wins up to the trained window...
		assert_eq!(effective_ctx_for(32768, Some(131072)), 32768);
		assert_eq!(effective_ctx_for(131072, Some(131072)), 131072);
		// ...and clamps to it beyond
		assert_eq!(effective_ctx_for(131072, Some(65536)), 65536);
		// a degenerate zero trained value must never yield a zero context
		// (load filters those out; the pure fn stays safe for any input)
		assert_eq!(effective_ctx_for(8192, Some(0)), 1);
	}

	#[test]
	fn the_truncation_budget_scales_with_the_effective_context() {
		use super::{effective_ctx_for, prompt_budget, DEFAULT_N_CTX, MAX_NEW_TOKENS_RESULT};
		let at_default = prompt_budget(effective_ctx_for(0, Some(131072)), MAX_NEW_TOKENS_RESULT);
		assert_eq!(
			at_default,
			(DEFAULT_N_CTX - MAX_NEW_TOKENS_RESULT - 64) as usize
		);
		let at_64k = prompt_budget(
			effective_ctx_for(65536, Some(131072)),
			MAX_NEW_TOKENS_RESULT,
		);
		assert_eq!(at_64k, (65536 - MAX_NEW_TOKENS_RESULT - 64) as usize);
		assert!(
			at_64k > at_default,
			"a larger setting shrinks what gets truncated"
		);
	}
}
