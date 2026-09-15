import { describe, expect, it } from "vitest";
import {
	addConversationMessage,
	removeLastConversationMessage,
	findMostRecentAssistantContent,
	findMostRecentUserContent
} from "./chat.js";

describe("addConversationMessage", () => {
	it("appends a user message and returns the new array", () => {
		let stored = [{ role: "assistant", content: "hi" }];
		const setConversation = (next) => {
			stored = next;
		};
		const next = addConversationMessage("hello", true, stored, setConversation);
		expect(next).toEqual([
			{ role: "assistant", content: "hi" },
			{ role: "user", content: "hello" }
		]);
		expect(stored).toBe(next);
	});

	it("appends an assistant message when isUser is falsy", () => {
		const next = addConversationMessage("hey", false, [], () => {});
		expect(next[0].role).toBe("assistant");
	});

	it("does not mutate the previous array", () => {
		const before = [{ role: "user", content: "a" }];
		addConversationMessage("b", false, before, () => {});
		expect(before).toEqual([{ role: "user", content: "a" }]);
	});

	it("reports the real new length (no stale closure guess)", () => {
		let stored = [];
		const setConversation = (next) => {
			stored = next;
		};
		// simulate the second call using the value the caller holds
		const first = addConversationMessage("a", true, stored, setConversation);
		const second = addConversationMessage("b", true, first, setConversation);
		expect(second.length).toBe(2);
	});
});

describe("removeLastConversationMessage", () => {
	it("removes and returns the last message content", () => {
		const conversation = [
			{ role: "user", content: "a" },
			{ role: "assistant", content: "b" }
		];
		let stored;
		const removed = removeLastConversationMessage(conversation, (next) => {
			stored = next;
		});
		expect(removed).toBe("b");
		// non-mutating: the shortened array is handed to the setter
		expect(conversation).toHaveLength(2);
		expect(stored).toHaveLength(1);
	});

	it("returns an empty string for an empty conversation", () => {
		expect(removeLastConversationMessage([], () => {})).toBe("");
	});
});

describe("findMostRecent*", () => {
	const conversation = [
		{ role: "assistant", content: "one" },
		{ role: "user", content: "two" },
		{ role: "assistant", content: "three" }
	];

	it("finds the last assistant message", () => {
		expect(findMostRecentAssistantContent(conversation)).toBe("three");
	});

	it("finds the last user message", () => {
		expect(findMostRecentUserContent(conversation)).toBe("two");
	});

	it("returns null when nothing matches", () => {
		expect(findMostRecentAssistantContent([{ role: "user", content: "x" }])).toBeNull();
	});
});
