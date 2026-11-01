import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { mockInvoke } from "@src/test/mock-tauri";
import IdeaResultContent from "./IdeaResultContent";

const idea = {
	id: "i1",
	title: "Title",
	result: "# Title\n\n## Body\nOriginal",
	result_json: [
		{ heading: "# Title", body: "" },
		{ heading: "## Body", body: "Original" }
	],
	transcript: [{ role: "user", content: "private" }]
};

beforeEach(() => window.history.pushState({}, "", "/idea?id=i1"));

describe("document editing", () => {
	it("copies the saved Markdown without the transcript", async () => {
		const user = userEvent.setup();
		mockInvoke({
			get_idea: () => idea,
			get_idea_children: () => ({ ideas: [] }),
			get_reactions: () => ({ sections: [], comments: [] })
		});
		render(<IdeaResultContent />);
		await user.click(await screen.findByRole("button", { name: "Copy as Markdown" }));
		expect(await navigator.clipboard.readText()).toBe(idea.result);
	});
	it("saves only the result and preserves the transcript", async () => {
		const user = userEvent.setup();
		mockInvoke({
			get_idea: () => idea,
			get_idea_children: () => ({ ideas: [] }),
			get_reactions: () => ({ sections: [], comments: [] }),
			set_idea_result: () => null
		});
		render(<IdeaResultContent />);
		await user.click(await screen.findByRole("button", { name: "Edit document" }));
		const editor = screen.getByLabelText("Document Markdown");
		await user.clear(editor);
		await user.type(editor, "Updated");
		await user.click(screen.getByRole("button", { name: "Save changes" }));
		await waitFor(() =>
			expect(vi.mocked(invoke).mock.calls).toContainEqual([
				"set_idea_result",
				{ ideaId: "i1", result: "Updated", expectedPreviousResult: idea.result }
			])
		);
	});

	it("offers no edit while feedback exists", async () => {
		mockInvoke({
			get_idea: () => idea,
			get_idea_children: () => ({
				ideas: [{ id: "f", created_at: "2026", result: "feedback" }]
			}),
			get_reactions: () => ({ sections: [], comments: [] })
		});
		render(<IdeaResultContent />);
		expect(await screen.findByRole("button", { name: "Copy as Markdown" })).toBeInTheDocument();
		expect(screen.queryByRole("button", { name: "Edit document" })).not.toBeInTheDocument();
	});

	it("shows the backend's refusal when the save is rejected", async () => {
		const user = userEvent.setup();
		mockInvoke({
			get_idea: () => idea,
			get_idea_children: () => ({ ideas: [] }),
			get_reactions: () => ({ sections: [], comments: [] }),
			set_idea_result: () => {
				throw "This document changed. Reopen the idea before editing.";
			}
		});
		render(<IdeaResultContent />);
		await user.click(await screen.findByRole("button", { name: "Edit document" }));
		await user.click(screen.getByRole("button", { name: "Save changes" }));
		expect(await screen.findByText(/This document changed/)).toBeInTheDocument();
		// the editor stays open with the user's text
		expect(screen.getByLabelText("Document Markdown")).toBeInTheDocument();
	});
});
