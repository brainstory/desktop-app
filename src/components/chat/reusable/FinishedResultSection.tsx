import { useEffect, useRef } from "react";
import GeneratingResult from "@src/components/chat/generating-result/GeneratingResult";
import StickyLoadingSection from "@src/components/chat/generating-result/StickyLoadingSection";

interface FinishedResultSectionProps {
	result: string;
	readyForFinish: boolean;
	/** the summary stream finished (render it as markdown) */
	isComplete: boolean;
	ideaId: string | undefined;
	/** saving the summary failed: show the error and a retry */
	saveError?: string | null;
	onRetrySave?: () => void;
	/** the model could only read the last part of the conversation */
	droppedTurns?: number;
	/** the stream stopped partway: offer retry / keep-draft */
	generationStopped?: boolean;
	onRetryGeneration?: () => void;
	onKeepDraft?: () => void;
	/** the latest generation status line */
	statusText?: string | null;
	/** a feedback result finished without its comment cards */
	cardsMissing?: boolean;
	/** the idea document was shortened to fit the context window */
	ideaCapped?: boolean;
}

export default function FinishedResultSection({
	result,
	readyForFinish,
	isComplete,
	ideaId,
	saveError,
	onRetrySave,
	droppedTurns,
	generationStopped,
	onRetryGeneration,
	onKeepDraft,
	statusText,
	cardsMissing,
	ideaCapped
}: FinishedResultSectionProps) {
	// the summary swap changes the whole page: put focus somewhere
	// meaningful instead of leaving it on the removed finish button
	const headingRef = useRef<HTMLDivElement | null>(null);
	useEffect(() => {
		headingRef.current?.focus();
	}, []);
	useEffect(() => {
		if (readyForFinish) {
			// the CTA lives inside StickyLoadingSection; focus it by its
			// stable id once the summary is saved
			document.getElementById("summary-cta")?.focus();
		}
	}, [readyForFinish]);
	return (
		<section>
			<div
				ref={headingRef}
				tabIndex={-1}
				className="relative mx-auto w-full max-w-5xl lg:px-24 md:px-12 px-2 p-2 focus:outline-none"
			>
				<div className="flex flex-col border border-stone-200 rounded-lg shadow my-4">
					<GeneratingResult summary={result} isComplete={isComplete} />
				</div>
			</div>
			<StickyLoadingSection
				isFinishedGenerating={readyForFinish}
				ideaId={ideaId}
				saveError={saveError}
				onRetrySave={onRetrySave}
				droppedTurns={droppedTurns}
				result={result}
				generationStopped={generationStopped}
				onRetryGeneration={onRetryGeneration}
				onKeepDraft={onKeepDraft}
				statusText={statusText}
				cardsMissing={cardsMissing}
				ideaCapped={ideaCapped}
			/>
		</section>
	);
}
