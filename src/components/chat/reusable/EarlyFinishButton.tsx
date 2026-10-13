import { CONVERSATION_STATE, type ConversationState } from "@src/const";
import Button from "@ds/Button";

interface EarlyFinishButtonProps {
	conversationState: ConversationState;
	/** ask for the summary before the usual minimum length */
	handleEarlyFinish: () => void;
	/** the session is being created/saved for the wrap-up */
	isWrappingUp: boolean;
	classes?: string;
}

/**
 * "Wrap up now" for a session below the usual minimum: more answers make
 * a richer summary, but stopping early is the user's call. The idea is
 * created on the way if the session has not saved one yet.
 */
export default function EarlyFinishButton({
	conversationState,
	handleEarlyFinish,
	isWrappingUp,
	classes = ""
}: EarlyFinishButtonProps) {
	const canFinish =
		conversationState === CONVERSATION_STATE.Start ||
		conversationState === CONVERSATION_STATE.Idle ||
		conversationState === CONVERSATION_STATE.ReadyToSendUserTranscript;
	return (
		<div className={`inline-block ${classes}`}>
			<p className="text-xs sm:text-sm text-stone-500 mb-1">
				More answers make a richer summary &mdash; wrap up any time.
			</p>
			<Button
				variant="bordered"
				disabled={!canFinish || isWrappingUp}
				onClick={handleEarlyFinish}
			>
				{isWrappingUp ? "Wrapping up…" : "That’s enough — summarize"}
			</Button>
		</div>
	);
}
