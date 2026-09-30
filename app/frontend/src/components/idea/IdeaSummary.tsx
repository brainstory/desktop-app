import ReactMarkdown from "react-markdown";
import LoadingAnimation from "@components/global/LoadingAnimation";

interface IdeaSummaryProps {
	content?: string | null;
}

export default function IdeaSummary({ content }: IdeaSummaryProps) {
	// explicit loading state instead of a markdown string used as a flag
	if (content == null) {
		return (
			<div className="p-5 pb-8 mx-auto">
				<LoadingAnimation text="Loading idea..." />
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
