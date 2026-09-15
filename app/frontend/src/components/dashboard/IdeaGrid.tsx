import { useEffect } from "react";
import IdeaCard from "@components/idea/IdeaCard";
import IdeaPlaceholder from "@components/idea/IdeaPlaceholder";
import DraftIdeaCard from "@components/idea/DraftIdeaCard";

import type { IdeaListItem } from "@src/types";

interface IdeaGridProps {
	userIdeas?: IdeaListItem[] | null;
}

export default function IdeaGrid({ userIdeas = [] }: IdeaGridProps) {
	useEffect(() => {}, [userIdeas]);

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
				{userIdeas.map((idea: IdeaListItem, index: number) => {
					// If summaryPreview is ... then is a draft summary
					if (idea.summaryPreview === "...") {
						return (
							<DraftIdeaCard
								key={`draft-idea-card-${index}-${idea.id}`}
								id={idea.id}
								createdAt={idea.createdAt}
								draftSummary={idea.draftSummary}
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
