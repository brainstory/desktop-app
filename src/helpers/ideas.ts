import type { CreatorInfo } from "@src/types";

/**
 * An "own" idea is one without creator attribution: imported ideas and
 * feedback carry the sender's creatorName, local ones never do. The one
 * definition shared by the dashboard and the idea page.
 */
export function isOwnIdea(idea: Pick<CreatorInfo, "creatorName">): boolean {
	return !idea.creatorName;
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
