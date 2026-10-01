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
