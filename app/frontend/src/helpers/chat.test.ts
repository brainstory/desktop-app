import { describe, expect, it } from "vitest";
import type { ChatMessage } from "@src/types";
import { TOPICS, CHAT_TYPE } from "@src/const";
import { getQuestionOfTheDay } from "@helpers/qotd";
import {
	handleStreamResult,
	addConversationMessage,
	removeLastConversationMessage,
	findMostRecentAssistantContent,
	findMostRecentUserContent,
	getFirstPrompt,
	groupTranscript
} from "./chat";

describe("getFirstPrompt", () => {
	const guide = (n: number) => TOPICS[n].prompt;

	it("uses the default greeting when no topic param is present", () => {
		expect(getFirstPrompt(CHAT_TYPE.ORIGINAL, { qotd: null, topic: null })).toBe(
			"Hi, how's it going? What's on your mind?"
		);
	});

	it("uses the default greeting for an empty or invalid topic param", () => {
		// "?topic=" used to resolve to Number("") === 0 -> TOPICS[0]
		expect(getFirstPrompt(CHAT_TYPE.ORIGINAL, { qotd: null, topic: "" })).toBe(
			"Hi, how's it going? What's on your mind?"
		);
		expect(getFirstPrompt(CHAT_TYPE.ORIGINAL, { qotd: null, topic: "abc" })).toBe(
			"Hi, how's it going? What's on your mind?"
		);
		expect(getFirstPrompt(CHAT_TYPE.ORIGINAL, { qotd: null, topic: "-1" })).toBe(
			"Hi, how's it going? What's on your mind?"
		);
		expect(getFirstPrompt(CHAT_TYPE.ORIGINAL, { qotd: null, topic: "1.5" })).toBe(
			"Hi, how's it going? What's on your mind?"
		);
	});

	it("uses the topic's prompt for a valid index, including 0", () => {
		expect(getFirstPrompt(CHAT_TYPE.ORIGINAL, { qotd: null, topic: "2" })).toBe(guide(2));
		expect(getFirstPrompt(CHAT_TYPE.ORIGINAL, { qotd: null, topic: "0" })).toBe(guide(0));
	});

	it("falls back to the greeting for an out-of-range index", () => {
		expect(getFirstPrompt(CHAT_TYPE.ORIGINAL, { qotd: null, topic: "99" })).toBe(
			"Hi, how's it going? What's on your mind?"
		);
	});

	it("qotd wins over topic", () => {
		expect(getFirstPrompt(CHAT_TYPE.ORIGINAL, { qotd: "", topic: "1" })).toBe(
			getQuestionOfTheDay()
		);
	});

	it("chatType overrides topic and qotd", () => {
		expect(getFirstPrompt(CHAT_TYPE.FEEDBACK, { qotd: "", topic: "1" })).toContain(
			"extend this idea"
		);
		expect(getFirstPrompt(CHAT_TYPE.DAILY_INTENT, { qotd: "", topic: "1" })).toBe(
			"Walk me through how you want your day to go."
		);
	});
});

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
		expect(groupTranscript(answered, { hideTrailingUnansweredQuestion: true })).toEqual([
			{ question: "q1", answer: "a1" }
		]);

		const unanswered: ChatMessage[] = [
			{ role: "assistant", content: "q1" },
			{ role: "user", content: "a1" },
			{ role: "assistant", content: "q2" }
		];
		expect(groupTranscript(unanswered, { hideTrailingUnansweredQuestion: true })).toEqual([
			{ question: "q1", answer: "a1" }
		]);
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
describe("handleStreamResult", () => {
	function makeStream() {
		let resolve!: (v: import("./chat").GenerationResult) => void;
		let reject!: (e: unknown) => void;
		const invokePromise = new Promise<import("./chat").GenerationResult>((res, rej) => {
			resolve = res;
			reject = rej;
		});
		const handle = {
			channel: {
				onmessage: (_event: import("./chat").StreamChunk) => {}
			},
			invokePromise
		};
		return {
			start: () => handle,
			emit: (event: import("./chat").StreamChunk) => handle.channel.onmessage(event),
			resolve,
			reject
		};
	}

	it("concatenates chunk events and lets cumulative replace", async () => {
		const { start, emit, resolve } = makeStream();
		const updates: string[] = [];
		const done = handleStreamResult(
			start,
			(m) => updates.push(m),
			async () => {}
		);
		emit({ type: "chunk", content: "hel" });
		emit({ type: "chunk", content: "lo" });
		emit({ type: "cumulative", content: "hello there" });
		resolve({});
		await done;
		expect(updates).toEqual(["hel", "hello", "hello there"]);
	});

	it("the final response overrides the streamed text", async () => {
		const { start, emit, resolve } = makeStream();
		const updates: string[] = [];
		const successes: string[] = [];
		const done = handleStreamResult(
			start,
			(m) => updates.push(m),
			async (message) => {
				successes.push(message);
			}
		);
		emit({ type: "chunk", content: "partial" });
		resolve({ response: "the real thing" });
		await done;
		expect(updates.at(-1)).toBe("the real thing");
		expect(successes).toEqual(["the real thing"]);
	});

	it("passes structured_result to the success callback", async () => {
		const { start, resolve } = makeStream();
		const structured: unknown[] = [];
		const done = handleStreamResult(
			start,
			() => {},
			async (_m, s) => {
				structured.push(s);
			}
		);
		resolve({ response: "x", structured_result: { items: 1 } });
		await done;
		expect(structured).toEqual([{ items: 1 }]);
	});

	it("calls onError and skips success when the invoke rejects", async () => {
		const { start, reject } = makeStream();
		const errors: unknown[] = [];
		let successRan = false;
		const done = handleStreamResult(
			start,
			() => {},
			async () => {
				successRan = true;
			},
			(err) => errors.push(err)
		);
		reject(new Error("endpoint down"));
		await done;
		expect(errors).toHaveLength(1);
		expect(successRan).toBe(false);
	});
});
