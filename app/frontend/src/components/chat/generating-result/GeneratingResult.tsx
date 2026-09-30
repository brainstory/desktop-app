import { useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";

/**
 * Markdown view for the streaming summary. Re-parsing the full markdown
 * tree on every streamed chunk is quadratic in the document length;
 * while text is still arriving we render plain text (cheap), and swap
 * to the formatted markdown once the stream completes.
 */
export default function GeneratingResult({ summary }: { summary: string }) {
	const [isStreaming, setIsStreaming] = useState(false);
	const prevRef = useRef(summary);

	useEffect(() => {
		// a "finished" render is one where the text stopped changing for
		// a frame - the final cumulative/final event lands before the
		// parent unmounts us for the finished view
		if (summary !== prevRef.current) {
			setIsStreaming(true);
			prevRef.current = summary;
			const t = window.setTimeout(() => setIsStreaming(false), 250);
			return () => window.clearTimeout(t);
		}
	}, [summary]);

	return (
		<div className="overflow-y-auto max-w-[1000px] mx-auto p-4 md:p-8">
			<div className="flex justify-center">
				{isStreaming ? (
					<p className="text-stone-700 whitespace-pre-wrap font-mono text-sm">
						{summary}
					</p>
				) : (
					<div className="prose prose-stone max-w-none">
						{/* We *need* to keep the "prose" class to process the markdown properly */}
						<ReactMarkdown>{summary}</ReactMarkdown>
					</div>
				)}
			</div>
		</div>
	);
}
