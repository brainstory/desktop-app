import { describe, expect, it } from "vitest";
import { safeParse } from "valibot";
import {
	FeedbackItemSchema,
	ImportShareResultSchema,
	ResultSectionsSchema,
	StructuredFeedbackSchema
} from "./schemas";

describe("ResultSectionsSchema", () => {
	it("accepts a valid document", () => {
		const parsed = safeParse(ResultSectionsSchema, [
			{ heading: "Core Thesis", body: "The idea in one line." },
			{ heading: "", body: "" }
		]);
		expect(parsed.success).toBe(true);
	});

	it("rejects a member with a missing field", () => {
		const parsed = safeParse(ResultSectionsSchema, [{ heading: "Core Thesis" }]);
		expect(parsed.success).toBe(false);
	});

	it("rejects a non-array document", () => {
		const parsed = safeParse(ResultSectionsSchema, { heading: "x", body: "y" });
		expect(parsed.success).toBe(false);
	});
});

describe("StructuredFeedbackSchema", () => {
	it("accepts a valid container (members are validated one by one)", () => {
		const parsed = safeParse(StructuredFeedbackSchema, { feedback_items: [{}, null, 3] });
		expect(parsed.success).toBe(true);
	});

	it("rejects a non-object document", () => {
		const parsed = safeParse(StructuredFeedbackSchema, "feedback");
		expect(parsed.success).toBe(false);
	});

	it("rejects a non-array feedback_items", () => {
		const parsed = safeParse(StructuredFeedbackSchema, { feedback_items: "none" });
		expect(parsed.success).toBe(false);
	});
});

describe("FeedbackItemSchema", () => {
	it("accepts a valid member and maps it to the comment shape", () => {
		const parsed = safeParse(FeedbackItemSchema, {
			oid_heading_text: "Core Thesis",
			feedback_text: "Good point.",
			matched_spans: [[0, 4]]
		});
		expect(parsed.success).toBe(true);
		if (parsed.success) {
			expect(parsed.output).toEqual({
				oidHeadingText: "Core Thesis",
				feedbackText: "Good point.",
				matchedSpans: [[0, 4]]
			});
		}
	});

	it("degrades a non-array matched_spans to []", () => {
		const parsed = safeParse(FeedbackItemSchema, {
			feedback_text: "Good point.",
			matched_spans: "nope"
		});
		expect(parsed.success).toBe(true);
		if (parsed.success) expect(parsed.output.matchedSpans).toEqual([]);
	});

	it("rejects a non-string heading", () => {
		const parsed = safeParse(FeedbackItemSchema, {
			oid_heading_text: 7,
			feedback_text: "Good point."
		});
		expect(parsed.success).toBe(false);
	});

	it("rejects empty feedback text", () => {
		const parsed = safeParse(FeedbackItemSchema, { feedback_text: "   " });
		expect(parsed.success).toBe(false);
	});
});

describe("ImportShareResultSchema", () => {
	it("accepts a valid result", () => {
		const parsed = safeParse(ImportShareResultSchema, {
			cancelled: false,
			kind: "idea",
			id: "i1",
			title: "Garage plan",
			author: "Ada"
		});
		expect(parsed.success).toBe(true);
	});

	it("rejects a missing cancelled flag", () => {
		const parsed = safeParse(ImportShareResultSchema, { kind: "idea" });
		expect(parsed.success).toBe(false);
	});

	it("rejects non-string pass-through fields", () => {
		const parsed = safeParse(ImportShareResultSchema, {
			cancelled: false,
			title: { nested: true }
		});
		expect(parsed.success).toBe(false);
	});
});
