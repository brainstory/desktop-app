import IdeaCard from "./IdeaCard";
import DraftIdeaCard from "./DraftIdeaCard";

import type { IdeaListItem } from "@src/types";

interface IdeaBranchesProps {
	kids?: IdeaListItem[];
	/** the idea these are feedback on: an unfinished feedback draft is
	 * resumed as a feedback chat on it */
	parentId?: string;
	/** a feedback draft was deleted from its card */
	onDraftDeleted?: (id: string) => void;
}

export default function IdeaBranches({ kids = [], parentId, onDraftDeleted }: IdeaBranchesProps) {
	if (kids.length === 0) {
		return <p className="mx-10 my-8 text-sm text-stone-500">No feedback on this idea yet.</p>;
	}
	return (
		<div className="mx-10 space-y-12">
			<div className="flex justify-center flex-wrap gap-4 mb-10">
				{kids.map((idea) =>
					// an unfinished feedback chat has no result page to open:
					// its card resumes the chat instead
					idea.isDraft && parentId ? (
						<DraftIdeaCard
							key={"draft-" + idea.id}
							id={idea.id}
							createdAt={idea.createdAt}
							draftSummary={idea.draftSummary}
							parentId={parentId}
							onDeleted={onDraftDeleted}
						/>
					) : (
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
					)
				)}
			</div>
		</div>
	);
}
