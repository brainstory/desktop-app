import { describe, expect, it, vi } from "vitest";
import { mockInvoke } from "@src/test/mock-tauri";
import { invoke } from "@tauri-apps/api/core";

import { getIdeaApi, getIdeaChildrenApi } from "./idea";
import { getAllIdeasApi } from "./user";
import { saveUserSettingsApi } from "./settings";
import { downloadModelApi } from "./models";

describe("getIdeaApi", () => {
	it("maps snake_case to camelCase including parent_idea", async () => {
		mockInvoke({
			get_idea: () => ({
				id: "i1",
				title: "T",
				type: "original",
				created_at: "2026-09-15T10:30:00",
				creator_email: null,
				creator_name: "Ada",
				is_unread: true,
				transcript: [],
				result: "the summary",
				shared_with_users: [],
				parent_idea: {
					id: "p1",
					title: "Parent",
					created_at: "2026-09-01T00:00:00",
					creator_name: "Grace"
				},
				result_json: []
			})
		});
		const idea = await getIdeaApi("i1");
		expect(idea.id).toBe("i1");
		expect(idea.summary).toBe("the summary");
		expect(idea.creatorName).toBe("Ada");
		expect(idea.isUnread).toBe(true);
		expect(idea.parentIdea?.id).toBe("p1");
		expect(idea.parentIdea?.creatorName).toBe("Grace");
	});

	it("renders a null parent as null, not an object", async () => {
		mockInvoke({ get_idea: () => ({ id: "i1" }) });
		const idea = await getIdeaApi("i1");
		expect(idea.parentIdea).toBeNull();
	});
});

describe("getIdeaChildrenApi", () => {
	// the raw item still carries the LLM's emoji `labels` (old stored
	// data); they must not reach the UI any more
	it("builds feedbackComments (without LLM labels) and a cleaned summaryPreview", async () => {
		mockInvoke({
			get_idea_children: () => ({
				ideas: [
					{
						id: "c1",
						title: "Feedback",
						result: "# Title\n\n## Point one\nbody text",
						created_at: "2026-09-15T10:30:00",
						structured_result: {
							feedback_items: [
								{
									oid_heading_text: "1##Point one",
									matched_spans: [],
									feedback_text: "good point",
									labels: [{ name: "Insight", emoji: "💡" }]
								}
							]
						}
					}
				]
			})
		});
		const children = await getIdeaChildrenApi("p1");
		expect(children).toHaveLength(1);
		expect(children[0]!.feedbackComments).toEqual([
			{
				oidHeadingText: "1##Point one",
				matchedSpans: [],
				feedbackText: "good point",
				itemIndex: 0
			}
		]);
		// first line (before \n\n) dropped, leading ## stripped
		expect(children[0]!.summaryPreview).toBe("Point one body text");
		expect(children[0]!.isDraft).toBe(false);
	});

	it("flags an unfinished feedback session as a draft with its last user message", async () => {
		mockInvoke({
			get_idea_children: () => ({
				ideas: [
					{
						id: "fd1",
						title: "",
						result: "",
						created_at: "2026-09-15T10:30:00",
						transcript: [
							{ role: "assistant", content: "What do you think?" },
							{ role: "user", content: "invite the neighbours" },
							{ role: "assistant", content: "Why?" }
						],
						structured_result: null
					}
				]
			})
		});
		const [draft] = await getIdeaChildrenApi("p1");
		expect(draft!.isDraft).toBe(true);
		expect(draft!.draftSummary).toBe("invite the neighbours");
		expect(draft!.feedbackComments).toEqual([]);
	});
});

describe("getIdeaChildrenApi item index", () => {
	it("keeps each comment's index in its feedback_items array", async () => {
		const item = (text: string) => ({
			oid_heading_text: "1##Point one",
			matched_spans: [],
			feedback_text: text
		});
		mockInvoke({
			get_idea_children: () => ({
				ideas: [
					{
						id: "c1",
						created_at: "",
						structured_result: { feedback_items: [item("a")] }
					},
					{
						id: "c2",
						created_at: "",
						structured_result: { feedback_items: [item("b"), item("c"), item("d")] }
					}
				]
			})
		});
		const children = await getIdeaChildrenApi("p1");
		expect(children[0]!.feedbackComments.map((c) => c.itemIndex)).toEqual([0]);
		expect(children[1]!.feedbackComments.map((c) => [c.feedbackText, c.itemIndex])).toEqual([
			["b", 0],
			["c", 1],
			["d", 2]
		]);
	});
});

describe("getIdeaChildrenApi malformed structured feedback (F01)", () => {
	// stored structured_result is unknown-typed: only the backend's own
	// serialization of the other fields is trustworthy. These cases pin
	// the malformed-data policy: skip unusable MEMBERS (never the whole
	// list), keep the child visible, and never renumber survivors
	// (comment reactions are keyed by itemIndex).
	it("a malformed feedback_items container does not reject the child list", async () => {
		mockInvoke({
			get_idea_children: () => ({
				ideas: [
					{
						id: "broken",
						created_at: "2026-10-02T12:00:00",
						result: "Feedback",
						structured_result: { feedback_items: {} }
					}
				]
			})
		});
		const children = await getIdeaChildrenApi("parent");
		expect(children).toHaveLength(1);
		expect(children[0]!.id).toBe("broken");
		expect(children[0]!.feedbackComments).toEqual([]);
	});

	it("keeps a malformed child beside a valid child and maps the valid one", async () => {
		mockInvoke({
			get_idea_children: () => ({
				ideas: [
					{
						id: "broken",
						created_at: "2026-10-02T12:00:00",
						result: "Feedback",
						structured_result: { feedback_items: {} }
					},
					{
						id: "valid",
						created_at: "2026-10-02T12:00:00",
						result: "Feedback",
						structured_result: {
							feedback_items: [
								{
									oid_heading_text: "1## Point one",
									matched_spans: [],
									feedback_text: "good point"
								}
							]
						}
					}
				]
			})
		});
		const children = await getIdeaChildrenApi("parent");
		expect(children).toHaveLength(2);
		expect(children[0]!.feedbackComments).toEqual([]);
		expect(children[1]!.feedbackComments).toEqual([
			{
				oidHeadingText: "1## Point one",
				matchedSpans: [],
				feedbackText: "good point",
				itemIndex: 0
			}
		]);
	});

	it("skips null, number and object members; later members keep their ORIGINAL itemIndex", async () => {
		// a reaction placed on "second" is keyed by itemIndex 3; skipping
		// the two unusable members before it must not move it to 1
		mockInvoke({
			get_idea_children: () => ({
				ideas: [
					{
						id: "c1",
						created_at: "2026-10-02T12:00:00",
						result: "Feedback",
						structured_result: {
							feedback_items: [
								null,
								{
									oid_heading_text: "1## A",
									matched_spans: [],
									feedback_text: "first"
								},
								9,
								{ oid_heading_text: "2## B", feedback_text: "second" },
								{ oid_heading_text: "3## C", feedback_text: { nested: true } }
							]
						}
					}
				]
			})
		});
		const [child] = await getIdeaChildrenApi("parent");
		expect(child!.feedbackComments).toEqual([
			{
				oidHeadingText: "1## A",
				matchedSpans: [],
				feedbackText: "first",
				itemIndex: 1
			},
			{
				oidHeadingText: "2## B",
				matchedSpans: [],
				feedbackText: "second",
				itemIndex: 3
			}
		]);
	});

	it("skips non-string heading/text fields but keeps a member without a heading", async () => {
		// absent/null oid_heading_text is a tolerated legacy shape
		// (aggregation logs and skips it); a NUMBER/OBJECT heading would
		// crash parseHeadingIndex at render time, and empty feedback
		// text has nothing to display - both are unusable members
		mockInvoke({
			get_idea_children: () => ({
				ideas: [
					{
						id: "c1",
						created_at: "2026-10-02T12:00:00",
						result: "Feedback",
						structured_result: {
							feedback_items: [
								{
									oid_heading_text: 7,
									matched_spans: [],
									feedback_text: "numbered"
								},
								{
									oid_heading_text: { a: 1 },
									matched_spans: [],
									feedback_text: "object heading"
								},
								{ oid_heading_text: "1## A", matched_spans: [], feedback_text: 5 },
								{ matched_spans: [], feedback_text: "anon" },
								{
									oid_heading_text: null,
									matched_spans: "not an array",
									feedback_text: "null heading"
								}
							]
						}
					}
				]
			})
		});
		const [child] = await getIdeaChildrenApi("parent");
		expect(child!.feedbackComments).toEqual([
			{
				oidHeadingText: undefined,
				matchedSpans: [],
				feedbackText: "anon",
				itemIndex: 3
			},
			{
				oidHeadingText: undefined,
				matchedSpans: [],
				feedbackText: "null heading",
				itemIndex: 4
			}
		]);
	});

	it("treats a non-object structured_result as no comments without throwing", async () => {
		for (const structured_result of [null, "nope", 42, [1, 2]]) {
			mockInvoke({
				get_idea_children: () => ({
					ideas: [{ id: "c1", created_at: "", result: "Feedback", structured_result }]
				})
			});
			const children = await getIdeaChildrenApi("parent");
			expect(children[0]!.feedbackComments).toEqual([]);
		}
	});

	it("an empty feedback_items array stays a valid document", async () => {
		mockInvoke({
			get_idea_children: () => ({
				ideas: [
					{
						id: "c1",
						created_at: "",
						result: "Feedback",
						structured_result: { feedback_items: [] }
					}
				]
			})
		});
		const children = await getIdeaChildrenApi("parent");
		expect(children[0]!.feedbackComments).toEqual([]);
	});
});

describe("getAllIdeasApi", () => {
	it("sets isDraft and draftSummary from the last user message", async () => {
		mockInvoke({
			get_all_ideas: () => ({
				ideas: [
					{
						id: "draft1",
						result: "",
						transcript: [
							{ role: "assistant", content: "hi" },
							{ role: "user", content: "last words" }
						]
					},
					{ id: "done1", result: "## Summary" }
				]
			})
		});
		const ideas = await getAllIdeasApi();
		expect(ideas[0]!.isDraft).toBe(true);
		expect(ideas[0]!.draftSummary).toBe("last words");
		expect(ideas[1]!.isDraft).toBe(false);
		expect(ideas[1]!.draftSummary).toBeUndefined();
	});

	it("maps nested feedback like top-level ideas, flagging unfinished drafts", async () => {
		mockInvoke({
			get_all_ideas: () => ({
				ideas: [
					{
						id: "p1",
						title: "Party",
						result: "## Summary",
						created_at: "2026-09-01T10:00:00",
						feedback: [
							{
								id: "fd1",
								title: "",
								result: "",
								type: "feedback",
								created_at: "2026-09-02T10:00:00",
								transcript: [
									{ role: "assistant", content: "What do you think?" },
									{ role: "user", content: "invite the neighbours" }
								]
							},
							{
								id: "f1",
								title: "Feedback",
								result: "## Thoughts",
								type: "feedback",
								created_at: "2026-09-01T11:00:00",
								creator_name: "Ada"
							}
						]
					}
				]
			})
		});
		const [idea] = await getAllIdeasApi();
		expect(idea!.feedback).toEqual([
			expect.objectContaining({
				id: "fd1",
				isDraft: true,
				createdAt: "2026-09-02T10:00:00",
				draftSummary: "invite the neighbours"
			}),
			expect.objectContaining({
				id: "f1",
				isDraft: false,
				createdAt: "2026-09-01T11:00:00",
				creatorName: "Ada",
				draftSummary: undefined
			})
		]);
	});
});

describe("saveUserSettingsApi", () => {
	it("sends only the provided fields", async () => {
		mockInvoke({ save_user_settings: () => ({ id: "settings" }) });
		await saveUserSettingsApi({ timezone: "Europe/Berlin" });
		const call = vi.mocked(invoke).mock.calls[0]!;
		expect(call[0]).toBe("save_user_settings");
		expect(call[1]).toEqual({ user: { timezone: "Europe/Berlin" } });
	});
});

describe("downloadModelApi", () => {
	it("passes a Channel as the onEvent argument", async () => {
		mockInvoke({ download_model: () => undefined });
		const { invokePromise } = downloadModelApi("gemma-4-E2B-qat");
		await invokePromise.catch(() => {}); // may reject; the arg is what matters
		const call = vi.mocked(invoke).mock.calls[0]!;
		expect(call[0]).toBe("download_model");
		expect(call[1]).toHaveProperty("modelId", "gemma-4-E2B-qat");
		expect(call[1]).toHaveProperty("onEvent");
	});
});
