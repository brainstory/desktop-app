import { getQueryParam } from "@helpers/helpers";
import { QUERY_PARAMS } from "@src/tauri/commands";
import { cn } from "@helpers/cn";
import type { ParentIdea } from "@components/chat/types";

interface ChatIntroTextProps {
	parentIdea?: ParentIdea | null;
	classes?: string;
}

export default function ChatIntroText({ parentIdea, classes }: ChatIntroTextProps) {
	const isDailyIntent = getQueryParam(QUERY_PARAMS.dailyIntent);

	let title = "New Brainstory";
	if (parentIdea?.id) {
		title = "Let's take this idea even further!";
	} else if (isDailyIntent) {
		title = "Let's set your daily intentions!";
	}

	return <h1 className={cn("my-auto mx-1 text-base text-stone-500", classes)}>{title}</h1>;
}
