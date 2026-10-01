import ReactMarkdown from "react-markdown";
import LoadingAnimation from "@components/global/LoadingAnimation";

interface IdeaSummaryProps {
	content?: string | null;
	/** Explicit loading flag: a null `content` after load is an idea
	 * without a summary, not a pending fetch. */
	isLoading?: boolean;
}

export default function IdeaSummary({ content, isLoading = false }: IdeaSummaryProps) {
	if (isLoading) {
		return (
			<div className="p-5 pb-8 mx-auto">
				<LoadingAnimation text="Loading idea..." />
			</div>
		);
	}
	if (content == null || content.trim() === "") {
		return (
			<div className="p-5 pb-8 mx-auto">
				<p className="text-sm text-stone-500">This idea has no summary yet.</p>
			</div>
		);
	}
	return (
		<div
			className={
				"p-5 pb-8 mx-auto prose prose-stone prose-headings:underline prose-headings:decoration-pink-500 prose-headings:text-xl prose-h1:no-underline prose-h1:text-xl " +
				"prose-p:leading-normal lg:prose-p:leading-loose"
			}
		>
			<ReactMarkdown>{content}</ReactMarkdown>
		</div>
	);
}
