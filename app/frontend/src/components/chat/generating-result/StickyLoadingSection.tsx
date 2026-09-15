import LoadingAnimation from "@components/global/LoadingAnimation";
import BorderedButton from "@ds/BorderedButton";

const headingStyle = "mb-4 lg:mb-5 font-bold text-stone-900 text-center text-xl lg:text-2xl";

interface StickyLoadingSectionProps {
	isFinishedGenerating: boolean;
	ideaId: string | undefined;
}

export default function StickyLoadingSection({ isFinishedGenerating, ideaId }: StickyLoadingSectionProps) {
	const title = isFinishedGenerating
		? "Finished! Saved your summary ✅"
		: "Hang tight! Writing your thoughts down...";

	return (
		<div className="sticky bottom-0 bg-white border-t px-7 py-9">
			<h1 className={headingStyle}>{title}</h1>
			{isFinishedGenerating ? (
				<div>
					<BorderedButton
						onClick={() => (window.location.href = `/idea?id=${ideaId}`)}
						classes="ml-auto"
					>
					See more &rarr;
				</BorderedButton>
				</div>
			) : (
				<LoadingAnimation text="Saving. Don't close this tab." />
			)}
		</div>
	);
}
