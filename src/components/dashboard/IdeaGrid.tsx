import IdeaCard from "@components/idea/IdeaCard";
import IdeaPlaceholder from "@components/idea/IdeaPlaceholder";
import DraftIdeaCard from "@components/idea/DraftIdeaCard";

import { useState } from "react";
import type { IdeaListItem } from "@src/types";

interface IdeaGridProps {
	userIdeas?: IdeaListItem[] | null;
}

/** One card in the grid: a top-level idea, or an unfinished feedback
 * draft together with the idea it gives feedback on. */
interface GridEntry {
	idea: IdeaListItem;
	parent?: IdeaListItem;
}

/**
 * The library lists feedback nested under its idea, never at the top
 * level, so an unfinished feedback chat would have no card to be resumed
 * from: lift each feedback draft out next to the other drafts. Newest
 * first, like the library itself (a stable sort, so ties keep its order).
 */
function gridEntries(ideas: IdeaListItem[]): GridEntry[] {
	const entries: GridEntry[] = ideas.map((idea) => ({ idea }));
	for (const parent of ideas) {
		for (const feedback of parent.feedback ?? []) {
			if (feedback.isDraft) entries.push({ idea: feedback, parent });
		}
	}
	const createdAt = (entry: GridEntry) => entry.idea.createdAt ?? "";
	return entries.sort((a, b) =>
		createdAt(a) < createdAt(b) ? 1 : createdAt(a) > createdAt(b) ? -1 : 0
	);
}

export default function IdeaGrid({ userIdeas = [] }: IdeaGridProps) {
	// locally-removed draft ids: a delete updates the grid in place
	// instead of reloading the whole page
	const [removed, setRemoved] = useState<Set<string>>(new Set());
	const visible = gridEntries(userIdeas ?? []).filter(({ idea }) => !removed.has(idea.id));

	if (userIdeas === null) {
		return (
			<div className="grid grid-cols-2 md:grid-cols-4 gap-4">
				{Array.from({ length: 4 }, (_, i) => (
					<IdeaPlaceholder key={`placeholder-grid-${i}`} />
				))}
			</div>
		);
	} else {
		return (
			<div className="flex flex-wrap items-stretch justify-center sm:justify-start gap-5 mx-auto">
				{visible.map(({ idea, parent }: GridEntry, index: number) => {
					if (idea.isDraft) {
						return (
							<DraftIdeaCard
								key={`draft-idea-card-${index}-${idea.id}`}
								id={idea.id}
								createdAt={idea.createdAt}
								draftSummary={idea.draftSummary}
								parentId={parent?.id}
								parentTitle={parent?.title}
								onDeleted={(deletedId) =>
									setRemoved((prev) => new Set(prev).add(deletedId))
								}
							/>
						);
					} else {
						return (
							<IdeaCard
								key={`idea-card-${index}-${idea.id}`}
								id={idea.id}
								title={idea.title}
								createdAt={idea.createdAt}
								creatorName={idea.creatorName}
								isUnread={idea.isUnread ?? false}
								feedback={idea.feedback ?? undefined}
								index={index}
							/>
						);
					}
				})}
			</div>
		);
	}
}
