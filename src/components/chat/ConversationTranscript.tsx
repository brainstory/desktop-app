import type { ChatMessage } from "@src/types";
import { groupTranscript } from "@helpers/chat";

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
	// Pairs are built from the interleaved conversation, so answers stay
	// attached to the right question even after "ask a different question".
	// The trailing question only hides when the user hasn't answered it yet
	// (it's rendered in the main chat area instead).
	const pairs = groupTranscript(qna, {
		transcribing: isTranscriptionRunning,
		hideTrailingUnansweredQuestion: !isLoadingCoachResponse && !isTranscriptionRunning
	});

	return (
		<div className="mx-auto w-full max-w-5xl overflow-y-auto flex flex-col gap-4">
			{pairs.map((pair, index) => (
				<div
					className="grid grid-cols-1 gap-4 border-b border-stone-200 text-sm tracking-tight leading-snug"
					key={`transcript-${index}`}
				>
					<div className="flex flex-col flex-shrink-0">
						<span className="font-medium text-black">{pair.question}</span>
					</div>
					<div className="lg:col-span-2 mb-4">
						<p
							className={`text-stone-500 ${pair.answer === "transcribing..." ? "italic" : ""}`}
						>
							{pair.answer ?? ""}
						</p>
					</div>
				</div>
			))}
		</div>
	);
}
