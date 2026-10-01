import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { mockInvoke } from "@src/test/mock-tauri";
import { REACTIONS } from "@src/const";

import { getReactionsApi, toggleCommentReactionApi, toggleSectionReactionApi } from "./reactions";

describe("reaction API wrappers", () => {
	it("get_reactions sends the idea id and passes sections and comments through", async () => {
		const sections = [{ sectionIndex: 2, emoji: "👍", mine: false, from: "Ana" }];
		const comments = [{ feedbackIdeaId: "f1", itemIndex: 3, emoji: "💡" }];
		mockInvoke({ get_reactions: () => ({ sections, comments }) });
		await expect(getReactionsApi("i1")).resolves.toEqual({ sections, comments });
		expect(vi.mocked(invoke)).toHaveBeenCalledWith("get_reactions", { ideaId: "i1" });
	});

	it("get_reactions tolerates missing lists", async () => {
		mockInvoke({ get_reactions: () => ({}) });
		await expect(getReactionsApi("i1")).resolves.toEqual({ sections: [], comments: [] });
	});

	it("toggle_section_reaction sends idea, section and emoji and returns the new state", async () => {
		mockInvoke({ toggle_section_reaction: () => true });
		await expect(toggleSectionReactionApi("i1", 2, "🚀")).resolves.toBe(true);
		expect(vi.mocked(invoke)).toHaveBeenCalledWith("toggle_section_reaction", {
			ideaId: "i1",
			sectionIndex: 2,
			emoji: "🚀"
		});
	});

	it("toggle_comment_reaction sends feedback idea, item index and emoji", async () => {
		mockInvoke({ toggle_comment_reaction: () => false });
		await expect(toggleCommentReactionApi("f1", 4, "❓")).resolves.toBe(false);
		expect(vi.mocked(invoke)).toHaveBeenCalledWith("toggle_comment_reaction", {
			feedbackIdeaId: "f1",
			itemIndex: 4,
			emoji: "❓"
		});
	});
});

describe("REACTIONS", () => {
	it("is the backend's fixed set of eight, with the emoji-presentation warning sign", () => {
		expect(REACTIONS.map((r) => r.emoji)).toEqual([
			"\u{1F44D}",
			"\u{1F44E}",
			"❓",
			"\u{1F4A1}",
			"\u{1F615}",
			"⚠️",
			"\u{1F4DA}",
			"\u{1F680}"
		]);
		expect(REACTIONS.map((r) => r.name)).toEqual([
			"agree",
			"disagree",
			"question",
			"suggestion",
			"confused",
			"error",
			"info",
			"action"
		]);
	});
});
