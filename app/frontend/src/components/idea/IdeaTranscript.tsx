import type { ChatMessage } from "@src/types";
import { groupTranscript } from "@helpers/chat";

export default function IdeaTranscript({
	transcript,
	title: _title
}: {
	transcript?: ChatMessage[] | null;
	title?: string | null;
}) {
	const pairs = groupTranscript(transcript ?? []);

	return (
		<div className="mx-10 divide-y-2 divide-stone-200">
			{pairs.map((pair, index) => (
				<div
					className="grid grid-cols-1 gap-4 py-6 lg:grid-cols-3 lg:py-12 first:pt-2"
					key={`transcript-${index}`}
				>
					<div className="flex flex-col flex-shrink-0 mb-6 lg:pr-12 md:mb-0">
						<span className="text-lg font-semibold leading-6 text-black font-display tracking-tight">
							{pair.question}
						</span>
					</div>
					<div className="lg:col-span-2">
						<p className="text-stone-500 text-sm">{pair.answer ?? ""}</p>
					</div>
				</div>
			))}
		</div>
	);
}
