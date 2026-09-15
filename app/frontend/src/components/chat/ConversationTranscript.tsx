import type { ChatMessage } from "@src/types";

interface ConversationTranscriptProps {
	qna?: ChatMessage[];
	isTranscriptionRunning?: boolean;
	isLoadingCoachResponse?: boolean;
}

export default function ConversationTranscript({
	qna = [],
	isTranscriptionRunning,
	isLoadingCoachResponse
}: ConversationTranscriptProps) {
	const splitConversation = (conversation: ChatMessage[]) => {
		let userContent: string[] = [];
		const assistantContent: string[] = [];
		conversation.forEach((message: ChatMessage) => {
			if (message.role === "user") {
				userContent.push(message.content);
			} else {
				assistantContent.push(message.content);
			}
		});
		if (isTranscriptionRunning) {
			userContent = [...userContent, "transcribing..."];
		} else if (!isLoadingCoachResponse) {
			assistantContent.pop();
		}

		return { assistantContent, userContent };
	};

	const { assistantContent, userContent } = splitConversation(qna);

	return (
		<div className="mx-auto w-full max-w-5xl overflow-y-auto flex flex-col gap-4">
			{assistantContent.map((question, index) => (
				<div
					className="grid grid-cols-1 gap-4 border-b border-stone-200 text-sm tracking-tight leading-snug"
					key={`transcript-${index}`}
				>
					<div className="flex flex-col flex-shrink-0">
						<span className="font-medium text-black font-display">{question}</span>
					</div>
					<div className="lg:col-span-2 mb-4">
						<p className="text-stone-500">
							{userContent[index] ? userContent[index] : ""}
						</p>
					</div>
				</div>
			))}
		</div>
	);
}
