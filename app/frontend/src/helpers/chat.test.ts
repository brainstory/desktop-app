import { describe, expect, it } from "vitest";
import type { ChatMessage } from "@src/types";
import {
	addConversationMessage,
	removeLastConversationMessage,
	findMostRecentAssistantContent,
	findMostRecentUserContent,
	groupTranscript
} from "./chat";

describe("addConversationMessage", () => {
	it("appends a user message and returns the new array", () => {
		let stored: ChatMessage[] = [{ role: "assistant", content: "hi" }];
		const setConversation = (next: ChatMessage[]) => {
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
		let stored: ChatMessage[] = [];
		const setConversation = (next: ChatMessage[]) => {
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

describe("groupTranscript", () => {
	it("pairs each question with the answer that follows it", () => {
		const conversation: ChatMessage[] = [
			{ role: "assistant", content: "q1" },
			{ role: "user", content: "a1" },
			{ role: "assistant", content: "q2" },
			{ role: "user", content: "a2" }
		];
		expect(groupTranscript(conversation)).toEqual([
			{ question: "q1", answer: "a1" },
			{ question: "q2", answer: "a2" }
		]);
	});

	it("keeps answers attached to the right question after a skip message", () => {
		const conversation: ChatMessage[] = [
			{ role: "assistant", content: "q1" },
			{ role: "user", content: "Ask me a different question!" },
			{ role: "assistant", content: "q2" },
			{ role: "user", content: "real answer" }
		];
		expect(groupTranscript(conversation)).toEqual([
			{ question: "q1", answer: undefined },
			{ question: "q2", answer: "real answer" }
		]);
	});

	it("keeps a trailing answered question, drops a trailing unanswered one on demand", () => {
		const answered: ChatMessage[] = [
			{ role: "assistant", content: "q1" },
			{ role: "user", content: "a1" }
		];
		expect(
			groupTranscript(answered, { hideTrailingUnansweredQuestion: true })
		).toEqual([{ question: "q1", answer: "a1" }]);

		const unanswered: ChatMessage[] = [
			{ role: "assistant", content: "q1" },
			{ role: "user", content: "a1" },
			{ role: "assistant", content: "q2" }
		];
		expect(
			groupTranscript(unanswered, { hideTrailingUnansweredQuestion: true })
		).toEqual([{ question: "q1", answer: "a1" }]);
	});

	it("shows a transcribing placeholder for the pending answer", () => {
		const conversation: ChatMessage[] = [
			{ role: "assistant", content: "q1" },
			{ role: "user", content: "a1" },
			{ role: "assistant", content: "q2" }
		];
		expect(groupTranscript(conversation, { transcribing: true })).toEqual([
			{ question: "q1", answer: "a1" },
			{ question: "q2", answer: "transcribing..." }
		]);
	});

	it("concatenates consecutive user messages into one answer", () => {
		const conversation: ChatMessage[] = [
			{ role: "assistant", content: "q1" },
			{ role: "user", content: "part one" },
			{ role: "user", content: "part two" }
		];
		expect(groupTranscript(conversation)).toEqual([
			{ question: "q1", answer: "part one part two" }
		]);
	});
});
