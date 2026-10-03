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
}

export default function FinishedResultSection({
	result,
	readyForFinish,
	isComplete,
	ideaId,
	saveError,
	onRetrySave
}: FinishedResultSectionProps) {
	return (
		<section>
			<div className="relative mx-auto w-full max-w-5xl lg:px-24 md:px-12 px-2 p-2">
				<div className="flex flex-col border border-stone-200 rounded-lg shadow my-4">
					<GeneratingResult summary={result} isComplete={isComplete} />
				</div>
			</div>
			<StickyLoadingSection
				isFinishedGenerating={readyForFinish}
				ideaId={ideaId}
				saveError={saveError}
				onRetrySave={onRetrySave}
			/>
		</section>
	);
}
