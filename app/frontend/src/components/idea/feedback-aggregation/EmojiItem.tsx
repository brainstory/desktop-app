import Avatar from "@ds/Avatar";
import Tooltip from "@ds/Tooltip";
import { cn } from "@helpers/cn";

interface EmojiItemProps {
	ideaId?: string | null;
	labels?: { name?: string; emoji: string }[];
	creatorEmail?: string | null;
	onReactionClick?: () => void;
	/** draw the reaction as an accent-coloured chip (the clickable
	 * reactions in the list); off inside the feedback card */
	isHighlighted?: boolean;
	labelsHasBorder?: boolean;
	creatorName?: string | null;
	style?: string;
}

export default function EmojiItem({
	labels = [],
	creatorEmail,
	onReactionClick,
	isHighlighted = true,
	labelsHasBorder = false,
	creatorName = null,
	style = ""
}: EmojiItemProps) {
	const avatarLetter = creatorName ? creatorName.charAt(0) : (creatorEmail || "?").charAt(0);

	// Interactive only when there is something to do: inside an already
	// clickable parent (the sidebar's feedback card button) it must be
	// plain content - a button inside a button is invalid.
	const Wrapper = onReactionClick ? "button" : "div";

	return (
		<Wrapper
			{...(onReactionClick
				? { type: "button" as const, onClick: () => onReactionClick() }
				: {})}
			className={cn("flex flex-col items-center relative", style)}
		>
			<div
				className={cn(
					"flex items-center",
					!labelsHasBorder && "p-1",
					isHighlighted &&
						"border rounded-md border-accent-400 bg-accent-50 hover:bg-accent-200"
				)}
			>
				<Tooltip text={creatorName ?? creatorEmail ?? undefined} position="left">
					<Avatar
						style="mr-1"
						id={creatorEmail ?? undefined}
						charToShow={avatarLetter}
						size={"6"}
					/>
				</Tooltip>
				<div
					className={cn(
						"flex w-max capitalize",
						labelsHasBorder && "border rounded-md p-1"
					)}
				>
					{labels.map((label, idx) => (
						<Tooltip key={label.name ?? idx} text={label.name} position="left">
							{label.emoji}
						</Tooltip>
					))}
				</div>
			</div>
		</Wrapper>
	);
}
