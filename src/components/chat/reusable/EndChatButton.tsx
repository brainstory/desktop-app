import { CONVERSATION_STATE, type ConversationState } from "@src/const";
import Button from "@ds/Button";

interface EndChatButtonProps {
	conversationState: ConversationState;
	handleGetResult: () => void;
	classes?: string;
}

export default function EndChatButton({
	conversationState,
	handleGetResult,
	classes = ""
}: EndChatButtonProps) {
	const isFinishing = conversationState === CONVERSATION_STATE.FinishWithResult;
	return (
		<div className={`inline-block ${classes}`}>
			<p className="text-xs sm:text-sm mb-1">Ready to end your session?</p>
			<Button
				variant="black"
				icon={isFinishing ? null : "exit-outline"}
				disabled={
					!(
						conversationState === CONVERSATION_STATE.Start ||
						conversationState === CONVERSATION_STATE.Idle ||
						conversationState === CONVERSATION_STATE.ReadyToSendUserTranscript
					)
				}
				onClick={() => handleGetResult()}
			>
				{isFinishing ? "Loading summary..." : "Generate Summary & Save"}
			</Button>
		</div>
	);
}
