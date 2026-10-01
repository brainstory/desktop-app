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
 * Section index of a feedback comment's `oid_heading_text`
 * ("<ordinal>##<heading text>", 1-based per the feedback JSON prompt).
 * The idea's result sections keep index 0 as the (empty) title slot, so
 * a valid ordinal is a positive integer below `headingCount`. Anything
 * else (empty, "0", "-1", "1.5", out of range) returns null.
 */
export function parseHeadingIndex(
	oidHeadingText: string | null | undefined,
	headingCount: number
): number | null {
	const ordinal = (oidHeadingText ?? "").split("#")[0]!.trim();
	if (!/^\d+$/.test(ordinal)) return null;
	const index = Number(ordinal);
	if (index < 1 || index >= headingCount) return null;
	return index;
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
