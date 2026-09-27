import { CONVERSATION_STATE } from "@src/const";
import BlackButton from "@ds/BlackButton";

interface EndChatButtonProps {
	conversationState: string;
	handleGetResult: () => void;
	classes?: string;
}

export default function EndChatButton({ conversationState, handleGetResult, classes = "" }: EndChatButtonProps) {
	const isFinishing = conversationState === CONVERSATION_STATE.FinishWithResult;
	return (
		<div className={`inline-block ${classes}`}>
			<p className="text-xs sm:text-sm mb-1">Ready to end your session?</p>
			<BlackButton
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
			</BlackButton>
		</div>
	);
}
