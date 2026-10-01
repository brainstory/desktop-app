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
}

export default function StickyLoadingSection({
	isFinishedGenerating,
	ideaId,
	saveError = null,
	onRetrySave
}: StickyLoadingSectionProps) {
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
					<div>
						<Button
							variant="bordered"
							onClick={() => (window.location.href = `/idea?id=${ideaId}`)}
							classes="ml-auto"
						>
							Open your idea &rarr;
						</Button>
					</div>
				</>
			) : (
				<>
					<h1 className={headingStyle}>Hang tight! Writing your thoughts down...</h1>
					<LoadingAnimation text="Saving your summary — keep the app open." />
					<div className="flex justify-center mt-2">
						<CancelGenerationButton />
					</div>
				</>
			)}
		</div>
	);
}
