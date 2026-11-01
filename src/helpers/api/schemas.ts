import * as v from "valibot";

/**
 * Runtime validation for the JSON blobs that live in SQLite as free
 * text: the idea result document, the structured feedback document and
 * the share-import result. The database content is model-generated,
 * imported from files other people wrote, or written by an older build,
 * so every read validates instead of casting - a malformed blob degrades
 * to the same fallback as a missing one instead of crashing the page.
 */

/** One section of an idea's stored result document. */
export const ResultSectionSchema = v.object({
	heading: v.string(),
	body: v.string()
});

/** The stored result document (`ideas.result_json`). */
export const ResultSectionsSchema = v.array(ResultSectionSchema);

/** A non-array `matched_spans` degrades to an empty list (display-only
 * field), exactly like the hand-written validation it replaces. */
const matchedSpans = v.pipe(
	v.unknown(),
	v.transform((spans): unknown[] =>
		Array.isArray(spans) ? spans.map((item: unknown) => item) : []
	),
	v.array(v.unknown())
);

/** One member of a structured feedback document's `feedback_items`.
 * `oid_heading_text` may be absent or null (the member stays a
 * comment); a non-string value makes the member invalid, so it is
 * skipped like before. */
export const FeedbackItemSchema = v.pipe(
	v.object({
		oid_heading_text: v.optional(v.nullable(v.string())),
		feedback_text: v.pipe(
			v.string(),
			v.check((text) => text.trim() !== "", "empty feedback text")
		),
		matched_spans: v.optional(matchedSpans, [])
	}),
	// whitespace-only text and non-object members are unusable, not
	// schema failures: the caller skips them one by one
	v.transform(({ oid_heading_text, feedback_text, matched_spans }) => ({
		oidHeadingText: typeof oid_heading_text === "string" ? oid_heading_text : undefined,
		feedbackText: feedback_text,
		matchedSpans: matched_spans
	}))
);

/** The structured feedback document (`ideas.structured_result`): an
 * object with a `feedback_items` array. */
export const StructuredFeedbackSchema = v.object({
	feedback_items: v.array(v.unknown())
});

/** The share-import result: the `cancelled` flag is backend-owned, the
 * string fields pass straight through from the imported file. */
export const ImportShareResultSchema = v.object({
	cancelled: v.boolean(),
	kind: v.optional(v.string()),
	duplicate: v.optional(v.boolean()),
	id: v.optional(v.string()),
	parent_id: v.optional(v.string()),
	title: v.optional(v.string()),
	author: v.optional(v.string())
});
