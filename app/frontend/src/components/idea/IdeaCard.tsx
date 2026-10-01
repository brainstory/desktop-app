import { formatISO8601ToHumanReadable } from "@helpers/helpers";
import type { IdeaListItem } from "@src/types";

interface IdeaCardProps {
	id: string;
	summaryPreview?: string;
	title?: string | null;
	createdAt?: string | null;
	creatorName?: string | null;
	creatorEmail?: string | null;
	isUnread?: boolean | null;
	shared?: boolean;
	feedback?: IdeaListItem[] | null | undefined;
	isFeedback?: boolean;
	index?: number;
}

export default function IdeaCard({
	id,
	title,
	createdAt,
	creatorName,
	isUnread = false,
	shared = false,
	feedback,
	isFeedback = false
}: IdeaCardProps) {
	const feedbackList = feedback ?? [];
	const humanReadableDate = formatISO8601ToHumanReadable(createdAt ?? "");
	// Feedback cards don't show shared / feedback sections. Cards without
	// feedback don't need the extra room the feedback stack effect reserves.
	const cardHeight = isFeedback
		? `min-h-[240px]`
		: feedbackList.length > 0
			? `min-h-[310px]`
			: `min-h-[240px]`;

	return (
		// One title link stretched over the card (its ::after covers the
		// card body) plus sibling actions raised above it: no interactive
		// element is ever nested inside another.
		<article
			className={`relative w-80 sm:w-[275px] ${cardHeight} group cursor-pointer bg-white rounded-lg border border-stone-200 shadow hover:shadow-lg hover:-translate-y-1 transition-transform`}
		>
			{isUnread && (
				<span className="pointer-events-none absolute top-3 right-3 z-10 bg-accent-700 text-white text-[10px] font-bold uppercase tracking-wide px-2 py-0.5 rounded-full shadow">
					New
				</span>
			)}
			<FeedbackStack feedback={feedback} />
			<div className="relative bg-white border border-stone-200 rounded-lg">
				<div className={`${cardHeight} p-6 rounded-lg`}>
					<div>
						{isFeedback && (
							<span className="inline-block py-1 mb-3 text-pink-600 font-semibold tracking-wide uppercase">
								Feedback
							</span>
						)}

						<h3 className="h-[64px] text-xl font-semibold tracking-tight text-stone-900 line-clamp-2 mb-3">
							<PinkIcon shared={shared} />
							<a
								href={`/idea?id=${id}`}
								className="after:absolute after:inset-0 after:rounded-lg"
							>
								{title}
							</a>
						</h3>

						<p className="mb-3 text-xs text-stone-500">{humanReadableDate}</p>

						<div className="group mb-4 flex items-center">
							<div className="flex items-center justify-center h-8 w-8 rounded-full ring-2 ring-white mr-2 bg-pink-100 text-pink-600 text-sm font-semibold">
								{(creatorName || "You").trim().charAt(0).toUpperCase()}
							</div>
							<div className="w-full truncate">
								<p className="text-xs font-medium text-stone-800">Created by</p>
								<p className="text-xs font-medium text-stone-800 truncate">
									{creatorName || "You"}
								</p>
							</div>
						</div>
					</div>
					{!isFeedback && feedbackList.length > 0 && (
						<div className="flex justify-end mt-4">
							<a
								href={`/idea?id=${id}&tab=feedback`}
								className="relative z-10 hover:underline font-bold w-[130px] text-center text-accent-700 uppercase p-1 text-xs rounded-full"
							>
								{feedbackList.length}{" "}
								{feedbackList.length > 1 ? "feedback items" : "feedback item"}
							</a>
						</div>
					)}
				</div>
			</div>
		</article>
	);
}

interface FeedbackStackProps {
	feedback?: IdeaListItem[] | null | undefined;
}

function FeedbackStack({ feedback = [] }: FeedbackStackProps) {
	return (
		<>
			{feedback &&
				feedback.length > 0 &&
				feedback.map((_feedbackItem: IdeaListItem, index: number) => {
					if (index > 2) {
						return null;
					}

					const sharedCardPart =
						"absolute left-1 w-full bg-stone-50 border border-stone-300 rounded-lg h-[310px] transition-all group-hover:rotate-0";

					const classNameBasedOnIndex: Record<number, string> = {
						0: `${
							(feedback ?? []).length > 1
								? "group-hover:translate-x-2 group-hover:bg-stone-200"
								: "group-hover:translate-x-1 group-hover:bg-stone-50"
						} -rotate-2 group-hover:top-2 ${sharedCardPart}`,
						1: `group-hover:translate-x-1 rotate-6 group-hover:top-1.5 group-hover:bg-stone-100 ${sharedCardPart}`,
						2: `rotate-2 group-hover:top-1 group-hover:bg-stone-50 ${sharedCardPart} group-hover:shadow`
					};

					return (
						// decorative layer (the explicit feedback link is the
						// action); never focusable or clickable on its own
						<div
							key={`${index}-feedback-card`}
							aria-hidden="true"
							className={`pointer-events-none ${classNameBasedOnIndex[index]}`}
						>
							<div className="rounded-lg"></div>
						</div>
					);
				})}
		</>
	);
}

function PinkIcon({ shared }: { shared?: boolean }) {
	const icon = shared ? "mail-outline" : "flash";

	// Inline (not floated): the title is a line-clamp box, which ignores
	// floats and would push the icon onto its own line above the title.
	return (
		<span className="inline-flex items-center justify-center align-middle mr-2 w-6 h-6 shrink-0 bg-pink-100 text-pink-600 rounded-full ring-4 ring-white">
			<ion-icon
				class="w-4 hydrated"
				name={icon}
				role="img"
				aria-label={shared ? "feedback message" : "idea highlight"}
			></ion-icon>
		</span>
	);
}
