import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

import { mockInvoke } from "@src/test/mock-tauri";
import IdeaResultContent from "./IdeaResultContent";

const resultJson = [
	{ heading: "", body: "" },
	{ heading: "# Title", body: "" },
	{ heading: "## Alpha", body: "alpha body" },
	{ heading: "## Beta", body: "beta body" }
];

function rawIdea(overrides: Record<string, unknown> = {}) {
	return {
		id: "i1",
		title: "My idea",
		result: "# Title\n\n## Alpha\nalpha body\n\n## Beta\nbeta body",
		result_json: resultJson,
		is_unread: false,
		...overrides
	};
}

function feedbackItem(oid: string | null, text: string) {
	return {
		oid_heading_text: oid,
		matched_spans: [],
		feedback_text: text,
		labels: [{ name: "agree", emoji: "👍" }]
	};
}

beforeEach(() => {
	window.history.pushState({}, "", "/idea?id=i1");
	vi.spyOn(console, "warn").mockImplementation(() => {});
});

afterEach(() => {
	window.history.pushState({}, "", "/");
	vi.restoreAllMocks();
});

describe("IdeaResultContent feedback heading references", () => {
	it("files comments only under positive ordinals that map to a real heading", async () => {
		mockInvoke({
			get_idea: () => rawIdea(),
			get_idea_children: () => ({
				ideas: [
					{
						id: "f1",
						created_at: "2026-09-01T10:00:00",
						creator_name: "Ada",
						structured_result: {
							feedback_items: [
								feedbackItem("", "empty ref"),
								feedbackItem(null, "null ref"),
								feedbackItem("-1## x", "negative ref"),
								feedbackItem("0## x", "title slot ref"),
								feedbackItem("9## x", "out of range ref"),
								feedbackItem("2## Alpha", "valid ref")
							]
						}
					}
				]
			})
		});
		render(<IdeaResultContent />);
		expect(await screen.findByText("valid ref")).toBeInTheDocument();
		for (const rejected of [
			"empty ref",
			"null ref",
			"negative ref",
			"title slot ref",
			"out of range ref"
		]) {
			expect(screen.queryByText(rejected)).not.toBeInTheDocument();
		}
		expect(console.warn).toHaveBeenCalled();
	});
});
