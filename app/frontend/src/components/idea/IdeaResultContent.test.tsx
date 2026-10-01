import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

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

const noReactions = { sections: [], comments: [] };

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
			get_reactions: () => noReactions,
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

describe("IdeaResultContent LLM labels", () => {
	it("never renders the emoji labels stored on old feedback items", async () => {
		mockViewport(1024);
		mockInvoke({
			get_idea: () => rawIdea(),
			get_reactions: () => noReactions,
			get_idea_children: () => ({
				ideas: [
					{
						id: "f1",
						created_at: "2026-09-01T10:00:00",
						creator_name: "Ada",
						structured_result: {
							feedback_items: [feedbackItem("2## Alpha", "nice alpha")]
						}
					}
				]
			})
		});
		render(<IdeaResultContent />);
		expect(await screen.findByText("nice alpha")).toBeInTheDocument();
		// the comment is still attached to its section (avatar button)...
		expect(screen.getByRole("button", { name: "Show feedback from Ada" })).toBeInTheDocument();
		// ...but the LLM-chosen label is gone everywhere
		expect(document.body.textContent).not.toContain("👍");
	});
});

describe("IdeaResultContent ownership", () => {
	it("an idea with only a creatorEmail is still own (no creatorName), so it can be exported", async () => {
		mockInvoke({
			get_idea: () => rawIdea({ creator_email: "me@example.com" }),
			get_reactions: () => noReactions,
			get_idea_children: () => ({ ideas: [] })
		});
		render(<IdeaResultContent />);
		expect(await screen.findByRole("button", { name: "Export" })).toBeInTheDocument();
	});

	it("an imported idea (creatorName set) is not own", async () => {
		mockInvoke({
			get_idea: () => rawIdea({ creator_name: "Ada" }),
			get_reactions: () => noReactions,
			get_idea_children: () => ({ ideas: [] })
		});
		render(<IdeaResultContent />);
		expect(await screen.findByText("Created by Ada")).toBeInTheDocument();
		expect(screen.queryByRole("button", { name: "Export" })).not.toBeInTheDocument();
	});
});

/** a feedback child as get_idea_children sends it */
function feedbackChild(id: string, result: string, overrides: Record<string, unknown> = {}) {
	return {
		id,
		title: result ? "Feedback" : "",
		result,
		type: "feedback",
		created_at: "2026-09-01T10:00:00",
		transcript: [
			{ role: "assistant", content: "What do you think?" },
			{ role: "user", content: `said in ${id}` }
		],
		structured_result: null,
		...overrides
	};
}

describe("IdeaResultContent feedback tab", () => {
	it("counts only finished feedback, not an unfinished draft", async () => {
		mockInvoke({
			get_idea: () => rawIdea(),
			get_reactions: () => noReactions,
			get_idea_children: () => ({
				ideas: [feedbackChild("fd1", ""), feedbackChild("f1", "## Thoughts")]
			})
		});
		render(<IdeaResultContent />);
		const tab = await screen.findByRole("tab", { name: "Feedback (1)" });
		expect(tab).toBeEnabled();
	});

	it("stays open (uncounted) when the only child is a draft, so it can be resumed", async () => {
		mockInvoke({
			get_idea: () => rawIdea(),
			get_reactions: () => noReactions,
			get_idea_children: () => ({ ideas: [feedbackChild("fd1", "")] })
		});
		render(<IdeaResultContent />);
		const tab = await screen.findByRole("tab", { name: "Feedback" });
		expect(tab).toBeEnabled();
		expect(screen.queryByRole("tab", { name: /Feedback \(/ })).not.toBeInTheDocument();
	});
});

describe("IdeaResultContent layout", () => {
	beforeEach(() => {
		mockInvoke({
			get_idea: () => rawIdea(),
			get_reactions: () => noReactions,
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

describe("IdeaResultContent section reactions", () => {
	const anaAgrees = {
		sections: [{ sectionIndex: 2, emoji: "👍", mine: false, from: "Ana" }],
		comments: []
	};

	function alphaReactions() {
		return screen.findByRole("group", { name: "Reactions to Alpha" });
	}

	it("shows others' reactions with count and sender", async () => {
		mockViewport(1024);
		mockInvoke({
			get_idea: () => rawIdea({ creator_name: "Bo" }),
			get_reactions: () => anaAgrees,
			get_idea_children: () => ({ ideas: [] })
		});
		render(<IdeaResultContent />);
		const chip = within(await alphaReactions()).getByRole("button", { name: "agree 👍, 1" });
		expect(chip).toHaveAttribute("aria-pressed", "false");
		expect(chip).toHaveAccessibleDescription("Ana");
		expect(vi.mocked(invoke)).toHaveBeenCalledWith("get_reactions", { ideaId: "i1" });
		// the other section has none
		expect(
			within(screen.getByRole("group", { name: "Reactions to Beta" })).queryByRole("button", {
				name: /👍/
			})
		).not.toBeInTheDocument();
	});

	it("the picker toggles the user's own reaction; others' stay", async () => {
		const user = userEvent.setup();
		mockViewport(1024);
		let on = false;
		const toggle = vi.fn(() => (on = !on));
		mockInvoke({
			get_idea: () => rawIdea({ creator_name: "Bo" }),
			get_reactions: () => anaAgrees,
			get_idea_children: () => ({ ideas: [] }),
			toggle_section_reaction: toggle
		});
		render(<IdeaResultContent />);
		const alpha = await alphaReactions();
		await within(alpha).findByRole("button", { name: "agree 👍, 1" });

		await user.click(within(alpha).getByRole("button", { name: "React" }));
		expect(
			within(alpha).getByText(
				"Your reactions are included when you export feedback on this idea."
			)
		).toBeInTheDocument();
		await user.click(within(alpha).getByRole("button", { name: "React with agree 👍" }));

		expect(toggle).toHaveBeenCalledExactlyOnceWith({
			ideaId: "i1",
			sectionIndex: 2,
			emoji: "👍"
		});
		const mine = await within(alpha).findByRole("button", { name: "agree 👍, 2" });
		expect(mine).toHaveAttribute("aria-pressed", "true");
		expect(mine).toHaveAccessibleDescription("You, Ana");

		// clicking the chip removes only the user's own reaction
		await user.click(mine);
		expect(toggle).toHaveBeenCalledTimes(2);
		const anaOnly = await within(alpha).findByRole("button", { name: "agree 👍, 1" });
		expect(anaOnly).toHaveAttribute("aria-pressed", "false");
		expect(anaOnly).toHaveAccessibleDescription("Ana");
	});

	it("rolls back and shows an error when the toggle fails", async () => {
		const user = userEvent.setup();
		mockViewport(1024);
		let reject: (reason: unknown) => void = () => {};
		mockInvoke({
			get_idea: () => rawIdea(),
			get_reactions: () => noReactions,
			get_idea_children: () => ({ ideas: [] }),
			toggle_section_reaction: () =>
				new Promise((_, rej) => {
					reject = rej;
				})
		});
		vi.spyOn(console, "error").mockImplementation(() => {});
		render(<IdeaResultContent />);
		const alpha = await alphaReactions();

		await user.click(within(alpha).getByRole("button", { name: "React" }));
		await user.click(within(alpha).getByRole("button", { name: "React with suggestion 💡" }));
		// optimistic
		expect(within(alpha).getByRole("button", { name: "suggestion 💡, 1" })).toHaveAttribute(
			"aria-pressed",
			"true"
		);

		await act(async () => reject("database is locked"));
		expect(
			within(alpha).queryByRole("button", { name: "suggestion 💡, 1" })
		).not.toBeInTheDocument();
		expect(await screen.findByText("Error: Could not save your reaction")).toBeInTheDocument();
	});

	it("a sidebar comment still scrolls to its section's reaction row", async () => {
		const user = userEvent.setup();
		mockViewport(1024);
		const scrolled: string[] = [];
		Element.prototype.scrollIntoView = function (this: Element) {
			scrolled.push(this.id);
		};
		mockInvoke({
			get_idea: () => rawIdea(),
			get_reactions: () => noReactions,
			get_idea_children: () => ({
				ideas: [
					{
						id: "f1",
						created_at: "2026-09-01T10:00:00",
						creator_name: "Ada",
						structured_result: { feedback_items: [feedbackItem("3## Beta", "on beta")] }
					}
				]
			})
		});
		render(<IdeaResultContent />);
		await user.click(await screen.findByRole("button", { name: /on beta/ }));
		expect(scrolled).toContain("emojiList-3");
		expect(
			within(document.getElementById("emojiList-3")!).getByRole("group", {
				name: "Reactions to Beta"
			})
		).toBeInTheDocument();
	});
});

describe("IdeaResultContent comment reactions", () => {
	function withComments(toggle: (args: unknown) => unknown) {
		mockInvoke({
			get_idea: () => rawIdea(),
			get_reactions: () => ({
				sections: [],
				comments: [{ feedbackIdeaId: "f1", itemIndex: 2, emoji: "📚" }]
			}),
			get_idea_children: () => ({
				ideas: [
					{
						id: "f1",
						created_at: "2026-09-01T10:00:00",
						creator_name: "Ada",
						structured_result: {
							feedback_items: [
								feedbackItem("", "dropped: no heading"),
								feedbackItem("3## Beta", "on beta"),
								feedbackItem("2## Alpha", "on alpha")
							]
						}
					}
				]
			}),
			toggle_comment_reaction: toggle
		});
	}

	it("toggles with the feedback idea id and the item's original index", async () => {
		const user = userEvent.setup();
		mockViewport(1024);
		const toggle = vi.fn(() => true);
		withComments(toggle);
		render(<IdeaResultContent />);

		// two comments by Ada: find each card's bar through its text
		const alphaCard = (await screen.findByText("on alpha")).closest<HTMLElement>("[id='f1']")!;
		const alphaBar = within(alphaCard).getByRole("group", {
			name: "Reactions to comment from Ada"
		});
		// preloaded reaction lands on item 2 ("on alpha"), not on "on beta"
		expect(within(alphaBar).getByRole("button", { name: "info 📚, 1" })).toHaveAttribute(
			"aria-pressed",
			"true"
		);
		const betaCard = screen.getByText("on beta").closest<HTMLElement>("[id='f1']")!;
		const betaBar = within(betaCard).getByRole("group", {
			name: "Reactions to comment from Ada"
		});
		expect(within(betaBar).queryByRole("button", { name: /📚/ })).not.toBeInTheDocument();

		await user.click(within(betaBar).getByRole("button", { name: "React" }));
		await user.click(within(betaBar).getByRole("button", { name: "React with agree 👍" }));
		expect(toggle).toHaveBeenCalledExactlyOnceWith({
			feedbackIdeaId: "f1",
			itemIndex: 1,
			emoji: "👍"
		});
		expect(await within(betaBar).findByRole("button", { name: "agree 👍, 1" })).toHaveAttribute(
			"aria-pressed",
			"true"
		);
	});

	it("rolls back and shows an error when the toggle fails", async () => {
		const user = userEvent.setup();
		mockViewport(1024);
		vi.spyOn(console, "error").mockImplementation(() => {});
		withComments(() => {
			throw "database is locked";
		});
		render(<IdeaResultContent />);
		const alphaCard = (await screen.findByText("on alpha")).closest<HTMLElement>("[id='f1']")!;
		const chip = await within(alphaCard).findByRole("button", { name: "info 📚, 1" });

		await user.click(chip);
		expect(await screen.findByText("Error: Could not save your reaction")).toBeInTheDocument();
		expect(within(alphaCard).getByRole("button", { name: "info 📚, 1" })).toHaveAttribute(
			"aria-pressed",
			"true"
		);
	});
});
