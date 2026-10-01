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
	it("builds feedbackComments and a cleaned summaryPreview", async () => {
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
				labels: [{ name: "Insight", emoji: "💡" }]
			}
		]);
		// first line (before \n\n) dropped, leading ## stripped
		expect(children[0]!.summaryPreview).toBe("Point one body text");
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
