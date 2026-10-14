import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { cleanStores } from "nanostores";

import { mockInvoke } from "@src/test/mock-tauri";
import { $aiStatus } from "@components/global/aiStatusStore";
import ChatApp from "./ChatApp";

// Rive needs a canvas + WASM runtime that jsdom doesn't have
vi.mock("@components/global/RivePencil", () => ({ default: () => null }));

const parent = {
	id: "party",
	title: "Idea for a Surprise Birthday Party",
	result: "# Idea for a Surprise Birthday Party\n\n## Plan\nkeep it secret",
	type: "original",
	created_at: "2026-09-01T10:00:00"
};

const draftTranscript = [
	{ role: "assistant", content: "What do you think of the idea?" },
	{ role: "user", content: "invite the neighbours" },
	{ role: "assistant", content: "Who else should come?" }
];

/** an unfinished feedback chat as get_idea returns it */
const feedbackDraft = {
	id: "fb-draft",
	title: "",
	result: "",
	type: "feedback",
	created_at: "2026-09-03T10:00:00",
	transcript: draftTranscript,
	parent_idea: parent
};

function calls(command: string) {
	return vi.mocked(invoke).mock.calls.filter(([c]) => c === command);
}

describe("ChatApp resuming a feedback draft", () => {
	beforeEach(() => {
		// the recorder's persisted input mode must not leak between tests
		localStorage.clear();
	});
	afterEach(() => {
		window.history.replaceState(null, "", "/");
		cleanup();
		cleanStores($aiStatus);
	});

	it("loads the draft into a feedback chat and keeps saving to the same draft", async () => {
		window.history.replaceState(null, "", "/chat?parentId=party&id=fb-draft");
		mockInvoke({
			get_runtime_status: () => ({ llm: { state: "ready" }, stt: { state: "ready" } }),
			get_idea: (args) => {
				const { ideaId } = args as { ideaId: string };
				if (ideaId === "fb-draft") return feedbackDraft;
				if (ideaId === "party") return parent;
				throw new Error(`idea ${ideaId} not found`);
			},
			generate_streaming_response: () => ({ response: "And the cake?" }),
			update_idea: (args) => ({ id: (args as { id: string }).id })
		});
		render(<ChatApp />);

		// the feedback layout, showing the restored conversation
		expect(await screen.findByText("Let's take this idea even further!")).toBeInTheDocument();
		expect(await screen.findByText("Who else should come?")).toBeInTheDocument();

		const user = userEvent.setup();
		await user.click(screen.getByText("Type instead"));
		await user.type(screen.getByLabelText("Type your response"), "my aunt{Enter}");
		expect(await screen.findByText("And the cake?")).toBeInTheDocument();

		// the coach got the whole resumed conversation as a feedback chat
		// on the parent idea
		const generateArgs = calls("generate_streaming_response")[0]![1] as {
			chatType: string;
			reactTo: string;
			messages: { content: string }[];
		};
		expect(generateArgs.chatType).toBe("feedback");
		expect(generateArgs.reactTo).toBe(parent.result);
		expect(generateArgs.messages.map((m) => m.content)).toEqual([
			...draftTranscript.map((m) => m.content),
			"my aunt"
		]);

		// later saves update the same draft row; no new idea is created
		await waitFor(() => expect(calls("update_idea").length).toBeGreaterThan(0));
		for (const [, args] of calls("update_idea")) {
			expect(args).toMatchObject({ id: "fb-draft" });
		}
		const lastSave = calls("update_idea").at(-1)![1] as {
			transcript: { content: string }[];
		};
		expect(lastSave.transcript.map((m) => m.content)).toEqual([
			...draftTranscript.map((m) => m.content),
			"my aunt",
			"And the cake?"
		]);
		expect(calls("create_idea")).toHaveLength(0);
		expect(new URLSearchParams(window.location.search).get("id")).toBe("fb-draft");
	});
});

describe("ChatApp daily intent log handling", () => {
	beforeEach(() => {
		localStorage.clear();
	});
	afterEach(() => {
		window.history.replaceState(null, "", "/");
		cleanup();
		cleanStores($aiStatus);
	});

	function mockDaily(handlers: Record<string, (args: unknown) => unknown> = {}) {
		mockInvoke({
			get_runtime_status: () => ({ llm: { state: "ready" }, stt: { state: "ready" } }),
			get_log_questions: () => ({
				log: [{ id: 1, text: "How did today feel?", label: "mood" }]
			}),
			submit_log: () => ({ id: "log-1" }),
			create_idea: () => ({ id: "idea-1" }),
			update_idea: (args) => ({ id: (args as { id: string }).id }),
			generate_streaming_response: () => ({ response: "and then?" }),
			...handlers
		});
	}

	it("blocks the mic and the create while today's status is unknown", async () => {
		let resolveStatus!: (v: unknown) => void;
		mockDaily({
			get_daily_status: () => new Promise((res) => (resolveStatus = res))
		});
		window.history.replaceState(null, "", "/chat?dailyIntent=true");
		render(<ChatApp />);

		// the input waits for the daily status (typing is the default
		// while the mic is blocked), and no idea is created
		expect(await screen.findByText("Loading today's status…")).toBeInTheDocument();
		// typing stays possible; sending waits
		expect(screen.getByRole("button", { name: "Send message" })).toHaveAttribute(
			"aria-disabled",
			"true"
		);

		await act(async () => {
			resolveStatus({ log_id: "log-9", streak: 1 });
			await Promise.resolve();
		});
		// the status arrived: the lock note is gone, a typed message can
		// be sent, and the modal stays closed
		await waitFor(() =>
			expect(screen.queryByText("Loading today's status…")).not.toBeInTheDocument()
		);
		await userEvent.type(screen.getByLabelText("Type your response"), "planned day");
		expect(screen.getByRole("button", { name: "Send message" })).not.toHaveAttribute(
			"aria-disabled",
			"true"
		);
		expect(screen.queryByText(/daily log isn.t filled in/)).not.toBeInTheDocument();
	});

	it("offers filling in today's log after the modal was dismissed", async () => {
		mockDaily({
			get_daily_status: () => ({ log_id: null, streak: 0 })
		});
		window.history.replaceState(null, "", "/chat?dailyIntent=true");
		render(<ChatApp />);

		// no log today: the modal opens; Escape dismisses it
		expect(await screen.findByRole("dialog", { name: "Daily Intent Log" })).toBeInTheDocument();
		await userEvent.keyboard("{Escape}");

		// the persistent strip replaces the silently-unlinked session
		const strip = await screen.findByText(/daily log isn.t filled in yet/i);
		expect(strip.closest("[role='status']")).toBeInTheDocument();
		expect(calls("create_idea")).toHaveLength(0);

		// "Fill it in" reopens the modal
		await userEvent.click(screen.getByRole("button", { name: "Fill it in" }));
		expect(await screen.findByRole("dialog", { name: "Daily Intent Log" })).toBeInTheDocument();
	});

	it("links the session's idea when the log is submitted after it exists", async () => {
		mockDaily({
			get_daily_status: () => ({ log_id: null, streak: 0 }),
			get_idea: () => ({
				id: "idea-1",
				transcript: [
					{ role: "assistant", content: "Walk me through your day." },
					{ role: "user", content: "so far so good" }
				]
			})
		});
		window.history.replaceState(null, "", "/chat?dailyIntent=true&id=idea-1");
		render(<ChatApp />);

		// no log today: the modal opens first, dismiss it to reach the strip
		expect(await screen.findByRole("dialog", { name: "Daily Intent Log" })).toBeInTheDocument();
		await userEvent.keyboard("{Escape}");
		expect(await screen.findByText(/daily log isn.t filled in yet/i)).toBeInTheDocument();
		// fill the log in from the strip
		await userEvent.click(screen.getByRole("button", { name: "Fill it in" }));
		const submit = await screen.findByRole("button", { name: "Submit" });
		await userEvent.click(submit);

		await waitFor(() => expect(calls("link_idea_log")).toHaveLength(1));
		expect(calls("link_idea_log")[0]![1]).toMatchObject({
			ideaId: "idea-1",
			logId: "log-1"
		});
	});
});
