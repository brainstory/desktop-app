import IdeaCard from "@components/idea/IdeaCard";
import IdeaPlaceholder from "@components/idea/IdeaPlaceholder";
import DraftIdeaCard from "@components/idea/DraftIdeaCard";

import { useState } from "react";
import type { IdeaListItem } from "@src/types";

interface IdeaGridProps {
	userIdeas?: IdeaListItem[] | null;
}

export default function IdeaGrid({ userIdeas = [] }: IdeaGridProps) {
	// locally-removed draft ids: a delete updates the grid in place
	// instead of reloading the whole page
	const [removed, setRemoved] = useState<Set<string>>(new Set());
	const visible = (userIdeas ?? []).filter((idea) => !removed.has(idea.id));

	if (userIdeas === null) {
		return (
			<div className="grid grid-cols-2 md:grid-cols-4 gap-4">
				{[...Array(4)].map((_, i) => (
					<IdeaPlaceholder key={`placeholder-grid-${i}`} />
				))}
			</div>
		);
	} else {
		return (
			<div className="flex flex-wrap items-stretch justify-center sm:justify-start gap-5 mx-auto">
				{visible.map((idea: IdeaListItem, index: number) => {
					if (idea.isDraft) {
						return (
							<DraftIdeaCard
								key={`draft-idea-card-${index}-${idea.id}`}
								id={idea.id}
								createdAt={idea.createdAt}
								draftSummary={idea.draftSummary}
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
