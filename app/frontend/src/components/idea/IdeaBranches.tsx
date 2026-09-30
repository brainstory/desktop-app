import IdeaCard from "./IdeaCard";

import type { IdeaListItem } from "@src/types";

interface IdeaBranchesProps {
	kids?: IdeaListItem[];
}

export default function IdeaBranches({ kids = [] }: IdeaBranchesProps) {
	if (kids.length === 0) {
		return <p className="mx-10 my-8 text-sm text-stone-500">No feedback on this idea yet.</p>;
	}
	return (
		<div className="mx-10 space-y-12">
			<div className="flex justify-center flex-wrap gap-4 mb-10">
				{kids.map((idea) => (
					<IdeaCard
						key={"card-" + idea.id}
						id={idea.id}
						title={idea.title}
						summaryPreview={idea.summaryPreview}
						createdAt={idea.createdAt}
						creatorEmail={idea.creatorEmail}
						creatorName={idea.creatorName}
						isUnread={idea.isUnread}
						shared={true}
						isFeedback={true}
					/>
				))}
			</div>
		</div>
	);
}
