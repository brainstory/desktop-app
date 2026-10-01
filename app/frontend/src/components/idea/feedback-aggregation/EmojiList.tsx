import type { FeedbackComment } from "@src/types";
import Avatar from "@ds/Avatar";
import Tooltip from "@ds/Tooltip";
import { cn } from "@helpers/cn";

/**
 * The people who commented on one document section, as avatar buttons.
 * Clicking one focuses that comment in the sidebar; past six, a "+N more"
 * button focuses the seventh.
 */
interface EmojiListProps {
	reactions?: FeedbackComment[];
	onReactionClick: (reaction: FeedbackComment) => void;
	isFocused?: boolean;
}

function commenterName(comment: FeedbackComment): string {
	return comment.creatorName ?? comment.creatorEmail ?? "someone";
}

function avatarLetter(comment: FeedbackComment): string {
	return (comment.creatorName ?? comment.creatorEmail ?? "?").charAt(0);
}

export default function EmojiList({ reactions = [], onReactionClick, isFocused }: EmojiListProps) {
	return (
		<div
			className={cn(
				"flex flex-wrap p-1 rounded-lg w-fit",
				isFocused && "outline outline-accent-500 outline-2 ease-in-out duration-300"
			)}
		>
			{reactions &&
				reactions.slice(0, 6).map((reaction: FeedbackComment, index: number) => {
					const name = commenterName(reaction);
					return (
						<Tooltip key={index} text={name}>
							<button
								type="button"
								aria-label={`Show feedback from ${name}`}
								className="m-1 p-1 rounded-md border border-accent-400 bg-accent-50 hover:bg-accent-200"
								onClick={() => onReactionClick(reaction)}
							>
								<Avatar
									id={reaction.creatorEmail ?? undefined}
									charToShow={avatarLetter(reaction)}
									size={"6"}
								/>
							</button>
						</Tooltip>
					);
				})}
			{reactions && reactions.length > 6 && (
				<button
					type="button"
					className="flex items-center"
					onClick={() => onReactionClick(reactions[6]!)}
				>
					<span className="text-gray-600 w-max">+{reactions.length - 6} more</span>
				</button>
			)}
		</div>
	);
}
