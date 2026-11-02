use super::super::catalog::{find_model, ModelKind, LLM_MODELS};
use super::{rollback_candidate, LLM_ROLLBACK_ON_SAME, STT_ROLLBACK_ON_SAME};

/// Two distinct whisper specs, looked up by id: the catalog order
/// changed once (Parakeet moved to the front) and these positional
/// indexes silently selected different models than their names claimed.
fn whisper_pair() -> (
	&'static crate::models::ModelSpec,
	&'static crate::models::ModelSpec,
) {
	let first = find_model("whisper-small-en", ModelKind::Stt).expect("catalog");
	let second = find_model("whisper-base-en", ModelKind::Stt).expect("catalog");
	(first, second)
}

#[test]
fn whisper_retries_the_same_model_llama_does_not() {
	let (small, tiny) = whisper_pair();
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
