import LoadingAnimation from "@components/global/LoadingAnimation";
import Button from "@ds/Button";
import { CancelGenerationButton } from "@components/chat/reusable/ChatStateNotification";

const headingStyle = "mb-4 lg:mb-5 font-bold text-stone-900 text-center text-xl lg:text-2xl";

interface StickyLoadingSectionProps {
	isFinishedGenerating: boolean;
	ideaId: string | undefined;
	/** saving the generated summary failed */
	saveError?: string | null;
	onRetrySave?: () => void;
	/** the model could only read the last part of the conversation */
	droppedTurns?: number;
	/** the finished text, for copying */
	result?: string;
	/** the stream stopped partway: offer retry / keep-draft */
	generationStopped?: boolean;
	onRetryGeneration?: () => void;
	onKeepDraft?: () => void;
}

/** One-line amber notice: the conversation was longer than the model's
 * context, so the oldest turns were left out of the summary. */
function TruncationNotice({ droppedTurns }: { droppedTurns: number }) {
	return (
		<p className="mb-4 text-sm text-amber-700 bg-amber-50 border border-amber-200 rounded-md px-3 py-2 text-center">
			The model could only read the last part of this conversation ({droppedTurns} earlier
			turns were left out). Shorter sessions, or a larger context window under Settings &gt;
			AI Models, avoid this.
		</p>
	);
}

export default function StickyLoadingSection({
	isFinishedGenerating,
	ideaId,
	saveError = null,
	onRetrySave,
	droppedTurns = 0,
	result = "",
	generationStopped = false,
	onRetryGeneration,
	onKeepDraft
}: StickyLoadingSectionProps) {
	const truncated = droppedTurns > 0;
	return (
		<div className="sticky bottom-0 bg-white border-t px-7 py-9">
			{saveError !== null ? (
				<div role="alert" className="text-center">
					<h1 className={headingStyle}>Couldn&rsquo;t save your summary</h1>
					<p className="text-stone-700 mb-4">
						{saveError} Your summary is still here &mdash; try saving it again.
					</p>
					<div className="flex justify-center">
						<Button variant="bordered" onClick={onRetrySave}>
							Try saving again
						</Button>
					</div>
				</div>
			) : generationStopped ? (
				<div role="alert" className="text-center">
					<h1 className={headingStyle}>Generation stopped early</h1>
					<p className="text-stone-700 mb-4">
						The model stopped partway through writing your summary. Try again, or keep
						what was written as your summary.
					</p>
					<div className="flex justify-center gap-2">
						<Button variant="bordered" onClick={onRetryGeneration}>
							Try again
						</Button>
						<Button variant="pink" onClick={onKeepDraft}>
							Keep this draft
						</Button>
					</div>
				</div>
			) : isFinishedGenerating ? (
				<>
					<h1 className={headingStyle}>
						<ion-icon
							name="checkmark-circle"
							class="hydrated w-6 h-6 mr-2 align-middle text-green-600"
							role="img"
							aria-label="Saved"
						></ion-icon>
						Finished! Saved your summary
					</h1>
					{truncated && <TruncationNotice droppedTurns={droppedTurns} />}
					<div className="flex justify-end gap-2">
						<Button
							variant="bordered"
							onClick={() => {
								void navigator.clipboard
									.writeText(result)
									.then(() => undefined)
									.catch(() => undefined);
							}}
						>
							Copy as Markdown
						</Button>
						<Button
							variant="bordered"
							onClick={() => (window.location.href = `/idea?id=${ideaId}`)}
						>
							Open your idea &rarr;
						</Button>
					</div>
				</>
			) : (
				<>
					<h1 className={headingStyle}>Hang tight! Writing your thoughts down...</h1>
					{truncated && <TruncationNotice droppedTurns={droppedTurns} />}
					<LoadingAnimation text="Saving your summary — keep the app open." />
					<div className="flex justify-center mt-2">
						<CancelGenerationButton />
					</div>
				</>
			)}
		</div>
	);
}
