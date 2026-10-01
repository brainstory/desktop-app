import { getQueryParam } from "@helpers/helpers";
import { cn } from "@helpers/cn";

interface ChatIntroTextProps {
	parentIdea?: { id?: string; title?: string | null } | null;
	classes?: string;
}

export default function ChatIntroText({ parentIdea, classes }: ChatIntroTextProps) {
	const isDailyIntent = getQueryParam("dailyIntent");

	let title = "New Brainstory";
	if (parentIdea?.id) {
		title = "Let's take this idea even further!";
	} else if (isDailyIntent) {
		title = "Let's set your daily intentions!";
	}

	return <h1 className={cn("my-auto mx-1 text-base text-stone-500", classes)}>{title}</h1>;
}
