import type { ReactNode } from "react";
import LoadingAnimation from "@components/global/LoadingAnimation";
import Button from "@ds/Button";
import { CONVERSATION_STATE, type ConversationState } from "@src/const";
import { cancelGenerationApi } from "@helpers/api/ai";

/** Cancels the in-flight local generation; the backend checks the token
 * between chunks, so the pending invoke rejects shortly after. */
export function CancelGenerationButton() {
	return (
		<Button
			variant="bordered"
			onClick={() => {
				cancelGenerationApi().catch((e) => console.error("cancel failed", e));
			}}
		>
			Cancel
		</Button>
	);
}

function RenderFromState({ conversationState }: { conversationState: ConversationState }) {
	const renderList: (string | ReactNode)[] = [];

	if (conversationState === CONVERSATION_STATE.WaitingForCoach) {
		renderList.push(
			<div
				key={`status-${conversationState}`}
				className="mx-auto text-center flex flex-col items-center gap-2"
			>
				<LoadingAnimation
					isVertical={true}
					text="sending message..."
					key={`state-${conversationState}`}
				/>
				<CancelGenerationButton />
			</div>
		);
	} else if (conversationState === CONVERSATION_STATE.TranscribingUser) {
		renderList.push(
			<div
				key={`status-${conversationState}`}
				className="mx-auto text-center flex flex-col leading-snug"
			>
				<LoadingAnimation
					isVertical={true}
					text={[
						"transcribing...",
						<br key="loading-line-break"></br>,
						"may take up to 30 seconds"
					]}
				/>
			</div>
		);
	}

	return renderList;
}

export default RenderFromState;
