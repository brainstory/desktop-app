import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
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
			generate_response: () => ({ response: "And the cake?" }),
			update_idea: (args) => ({ id: (args as { id: string }).id })
		});
		render(<ChatApp />);

		// the feedback layout, showing the restored conversation
		expect(await screen.findByText("Let's take this idea even further!")).toBeInTheDocument();
		expect(await screen.findByText("Who else should come?")).toBeInTheDocument();

		const user = userEvent.setup();
		await user.click(screen.getByText("Not in a place to talk out-loud?"));
		await user.type(screen.getByLabelText("Type your response"), "my aunt{Enter}");
		expect(await screen.findByText("And the cake?")).toBeInTheDocument();

		// the coach got the whole resumed conversation as a feedback chat
		// on the parent idea
		const generateArgs = calls("generate_response")[0]![1] as {
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
