use super::super::catalog::{LLM_MODELS, STT_MODELS};
use super::{rollback_candidate, LLM_ROLLBACK_ON_SAME, STT_ROLLBACK_ON_SAME};

#[test]
fn whisper_retries_the_same_model_llama_does_not() {
	let small = &STT_MODELS[1];
	let tiny = &STT_MODELS[2];
	// a failed reload of the active whisper model gets a retry
	// (339b19e): its engine was already dropped
	let retry = rollback_candidate(Some(small.clone()), small, STT_ROLLBACK_ON_SAME);
	assert_eq!(retry.map(|s| s.id), Some(small.id));
	// switching whisper models rolls back to the previous one
	let back = rollback_candidate(Some(tiny.clone()), small, STT_ROLLBACK_ON_SAME);
	assert_eq!(back.map(|s| s.id), Some(tiny.id));

	// llama: an identical reload fails the same way, so no retry...
	let gemma = &LLM_MODELS[0];
	let other = &LLM_MODELS[1];
	assert!(rollback_candidate(Some(gemma.clone()), gemma, LLM_ROLLBACK_ON_SAME).is_none());
	// ...but a failed switch still restores the previous model
	let back = rollback_candidate(Some(other.clone()), gemma, LLM_ROLLBACK_ON_SAME);
	assert_eq!(back.map(|s| s.id), Some(other.id));

	// nothing was loaded before: nothing to restore
	assert!(rollback_candidate(None, small, STT_ROLLBACK_ON_SAME).is_none());
}
