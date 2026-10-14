import type { CreatorInfo, IdeaListItem } from "@src/types";

/**
 * An "own" idea is one without creator attribution: imported ideas and
 * feedback carry the sender's creatorName, local ones never do. The one
 * definition shared by the dashboard and the idea page.
 */
export function isOwnIdea(idea: Pick<CreatorInfo, "creatorName">): boolean {
	return !idea.creatorName;
}

/** Sort orders for the library toolbar. */
export type LibrarySort = "newest" | "oldest" | "title" | "unread";
/** Filter chips for the library toolbar. */
export type LibraryFilter = "drafts" | "unread" | "shared" | "mine";

export const LIBRARY_SORTS: { value: LibrarySort; label: string }[] = [
	{ value: "newest", label: "Newest first" },
	{ value: "oldest", label: "Oldest first" },
	{ value: "title", label: "Title A-Z" },
	{ value: "unread", label: "Unread first" }
];

export const LIBRARY_FILTERS: { value: LibraryFilter; label: string }[] = [
	{ value: "drafts", label: "Drafts" },
	{ value: "unread", label: "Unread" },
	{ value: "shared", label: "Shared with me" },
	{ value: "mine", label: "Mine" }
];

const sortByCreated = (a: IdeaListItem, b: IdeaListItem, dir: 1 | -1): number => {
	const at = a.createdAt ?? "";
	const bt = b.createdAt ?? "";
	return at < bt ? -dir : at > bt ? dir : 0;
};

/**
 * The library's whole view, derived client-side (all data is already
 * loaded). `query` matches title, summary preview, draft summary or
 * creator, case-insensitively. Chips combine with AND semantics; the
 * drafts/unread chips keep a parent whose nested feedback matches, so
 * the card the draft is reachable from stays in view. The sorts are
 * stable, so ties keep their previous order.
 */
export function filterAndSortIdeas(
	ideas: IdeaListItem[],
	query: string,
	filters: LibraryFilter[],
	sort: LibrarySort
): IdeaListItem[] {
	const q = query.trim().toLowerCase();
	const fieldsOf = (item: IdeaListItem) => [
		item.title,
		item.summaryPreview,
		item.draftSummary,
		item.creatorName
	];
	const matchesQuery = (idea: IdeaListItem): boolean => {
		if (!q) return true;
		// nested feedback drafts are only reachable through their parent
		// card, so their text keeps that card findable
		const children = (idea.feedback ?? []).flatMap(fieldsOf);
		return [...fieldsOf(idea), ...children].some(
			(field) => typeof field === "string" && field.toLowerCase().includes(q)
		);
	};
	const has = (filter: LibraryFilter) => filters.includes(filter);
	const passesChips = (idea: IdeaListItem): boolean => {
		if (
			has("drafts") &&
			!(idea.isDraft || (idea.feedback ?? []).some((child) => child.isDraft))
		) {
			return false;
		}
		if (
			has("unread") &&
			!(idea.isUnread || (idea.feedback ?? []).some((child) => child.isUnread))
		) {
			return false;
		}
		if (has("shared") && isOwnIdea(idea)) return false;
		if (has("mine") && !isOwnIdea(idea)) return false;
		return true;
	};

	const filtered = ideas.filter((idea) => matchesQuery(idea) && passesChips(idea));
	switch (sort) {
		case "oldest":
			filtered.sort((a, b) => sortByCreated(a, b, 1));
			break;
		case "title":
			filtered.sort(
				(a, b) => (a.title ?? "").localeCompare(b.title ?? "") || sortByCreated(a, b, -1)
			);
			break;
		case "unread":
			filtered.sort(
				(a, b) =>
					Number(Boolean(b.isUnread)) - Number(Boolean(a.isUnread)) ||
					sortByCreated(a, b, -1)
			);
			break;
		default:
			filtered.sort((a, b) => sortByCreated(a, b, -1));
	}
	return filtered;
}

/**
 * Markdown with its trailing "## Notes" section removed. The idea page
 * shows notes as an editable list instead of a rendered section; the
 * last such heading wins, because notes are only ever built at the end
 * of the document (mirrors the backend's strip).
 */
export function stripNotesSection(markdown: string): string {
	const lines = markdown.split("\n");
	let cut = -1;
	for (let i = 0; i < lines.length; i++) {
		if (lines[i]!.trim() === "## Notes") {
			cut = i;
		}
	}
	if (cut === -1) {
		return markdown.trim();
	}
	return lines.slice(0, cut).join("\n").trimEnd();
}

/**
 * Section index (into the idea's result sections: [title slot, "# Title",
 * "## ...", ...]) of a feedback comment's `oid_heading_text`,
 * "<number>##<heading text>". Placed by heading text first - it names the
 * section unambiguously - and otherwise by the number, which counts only
 * the `##` sections (first `##` = 1), as the feedback prompt defines it and
 * as the model numbers them. Using the number as a raw array index put
 * every comment one section too early (on the title). Returns null when
 * neither identifies a section.
 */
export function parseHeadingIndex(
	oidHeadingText: string | null | undefined,
	sections: { heading?: string | null }[]
): number | null {
	const raw = oidHeadingText ?? "";
	const hashes = raw.indexOf("##");
	const numberPart = (hashes === -1 ? raw : raw.slice(0, hashes)).trim();
	const textPart = hashes === -1 ? "" : normalizeHeading(raw.slice(hashes));

	// indexes of the "## " sections, in order
	const subsections = sections.flatMap((s, i) =>
		(s.heading ?? "").trimStart().startsWith("## ") ? [i] : []
	);

	if (textPart) {
		const matches = sections.flatMap((s, i) =>
			normalizeHeading(s.heading ?? "") === textPart ? [i] : []
		);
		// a title sharing its text with a section: prefer the section
		const sub = matches.filter((i) => subsections.includes(i));
		if (sub.length > 0) return sub[0]!;
		if (matches.length > 0) return matches[0]!;
	}

	if (!/^\d+$/.test(numberPart)) return null;
	const ordinal = Number(numberPart);
	if (ordinal < 1 || ordinal > subsections.length) return null;
	return subsections[ordinal - 1]!;
}

function normalizeHeading(heading: string): string {
	return heading
		.replace(/^\s*#+\s*/, "")
		.trim()
		.toLowerCase();
}

/**
 * Distill a result document into a short preview line (library grid,
 * feedback cards).
 * Total (never crashes on missing/empty results) and pure, so it can be
 * unit-tested.
 */
/** Body text for the library card: the document without its `#` title
 * line or markdown markers, capped so the 300-character SQL preview
 * window always suffices (the card clamps to two lines regardless). */
export function cardSummaryPreview(str: unknown): string {
	if (typeof str !== "string" || str === "") return "";
	const lines = str
		.split("\n")
		.map((line) => line.trim())
		.filter((line) => line !== "" && line !== "---");
	// the leading `#` title is shown on the card already; section
	// headings keep their text, only the markers go
	const bodyStart = lines[0]?.startsWith("#") ? 1 : 0;
	const text = lines
		.slice(bodyStart)
		.map((line) =>
			line
				.replace(/^#{1,6}\s*/, "")
				.replace(/^[-*+]\s+/, "")
				.replace(/^>\s*/, "")
				.replace(/\*\*(.+?)\*\*/g, "$1")
				.replace(/\*(.+?)\*/g, "$1")
				.replace(/`([^`]*)`/g, "$1")
		)
		.join(" ")
		.replace(/\s+/g, " ")
		.trim();
	return text.length > 220 ? `${text.slice(0, 220).trimEnd()}…` : text;
}

export function stripResultPreview(str: unknown): string {
	// remove the first line before the first \n\n,
	// and if the next line starts with ##, remove the ##
	// then replace all newlines with spaces
	if (typeof str !== "string" || str === "") return "";
	const removedFirstLine = str.includes("\n\n") ? str.substring(str.indexOf("\n\n") + 2) : str;
	const removedFirstLineAndHash = removedFirstLine.replace(/^##/, "");
	const removedNewLines = removedFirstLineAndHash.replace(/\n/g, " ");
	const trimmed = removedNewLines.trim();
	if (!trimmed) return "";
	// only append the ellipsis when something was actually cut off
	return trimmed.length > 100 ? trimmed.substring(0, 100).trim() + "..." : trimmed;
}

/**
 * Preview line for an unfinished (draft) session: its last non-empty user
 * message. `undefined` for a finished idea (non-empty result) or a draft
 * with nothing said yet.
 */
export function draftSummaryOf(
	result: string | null | undefined,
	transcript: { role: string; content: string }[] | null | undefined
): string | undefined {
	if (result) return undefined;
	return transcript?.filter((message) => message.role === "user" && message.content !== "").pop()
		?.content;
}

/** The finished feedback (it has a result): what feedback counts and
 * stacks show. Unfinished drafts are listed on their own, as drafts. */
export function finishedFeedback<T extends { isDraft?: boolean | null }>(
	feedback: T[] | null | undefined
): T[] {
	return (feedback ?? []).filter((item) => !item.isDraft);
}
