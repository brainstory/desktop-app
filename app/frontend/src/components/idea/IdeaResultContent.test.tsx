import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";

import { mockInvoke } from "@src/test/mock-tauri";
import { mockViewport } from "@src/test/match-media";
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

describe("IdeaResultContent ownership", () => {
	it("an idea with only a creatorEmail is still own (no creatorName), so it can be exported", async () => {
		mockInvoke({
			get_idea: () => rawIdea({ creator_email: "me@example.com" }),
			get_idea_children: () => ({ ideas: [] })
		});
		render(<IdeaResultContent />);
		expect(await screen.findByRole("button", { name: "Export" })).toBeInTheDocument();
	});

	it("an imported idea (creatorName set) is not own", async () => {
		mockInvoke({
			get_idea: () => rawIdea({ creator_name: "Ada" }),
			get_idea_children: () => ({ ideas: [] })
		});
		render(<IdeaResultContent />);
		expect(await screen.findByText("Created by Ada")).toBeInTheDocument();
		expect(screen.queryByRole("button", { name: "Export" })).not.toBeInTheDocument();
	});
});

describe("IdeaResultContent layout", () => {
	beforeEach(() => {
		mockInvoke({
			get_idea: () => rawIdea(),
			get_idea_children: () => ({ ideas: [] })
		});
	});

	it("uses the document + sidebar view above 768px and the plain summary at or below", async () => {
		const viewport = mockViewport(1024);
		render(<IdeaResultContent />);
		expect(await screen.findByText("All Feedback Comments")).toBeInTheDocument();

		act(() => viewport.setWidth(768));
		expect(screen.queryByText("All Feedback Comments")).not.toBeInTheDocument();
		expect(screen.getByText("alpha body")).toBeInTheDocument();

		act(() => viewport.setWidth(1200));
		expect(screen.getByText("All Feedback Comments")).toBeInTheDocument();
	});

	it("does not listen to every window resize", async () => {
		mockViewport(1024);
		const addListener = vi.spyOn(window, "addEventListener");
		render(<IdeaResultContent />);
		await screen.findByText("All Feedback Comments");
		expect(addListener.mock.calls.some(([type]) => type === "resize")).toBe(false);
	});
});
