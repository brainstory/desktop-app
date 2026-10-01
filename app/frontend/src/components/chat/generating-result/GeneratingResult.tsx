import ReactMarkdown from "react-markdown";

/**
 * Markdown view for the streaming summary. Re-parsing the full markdown
 * tree on every streamed chunk is quadratic in the document length;
 * while text is still arriving we render plain text (cheap), and swap
 * to the formatted markdown once the stream has completed. Completion
 * is an explicit signal from the generation call, not a quiet period:
 * slow generation pauses between chunks and would flip back and forth.
 */
export default function GeneratingResult({
	summary,
	isComplete
}: {
	summary: string;
	/** the generation finished: the summary is final */
	isComplete: boolean;
}) {
	return (
		<div className="overflow-y-auto max-w-[1000px] mx-auto p-4 md:p-8">
			<div className="flex justify-center">
				{isComplete ? (
					<div className="prose prose-stone max-w-none">
						{/* We *need* to keep the "prose" class to process the markdown properly */}
						<ReactMarkdown>{summary}</ReactMarkdown>
					</div>
				) : (
					<p className="text-stone-700 whitespace-pre-wrap font-mono text-sm">
						{summary}
					</p>
				)}
			</div>
		</div>
	);
}
