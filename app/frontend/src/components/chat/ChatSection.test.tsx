import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { ChatSection } from "./ChatSection";

// Rive needs a canvas + WASM runtime that jsdom doesn't have
vi.mock("@components/global/RivePencil", () => ({ default: () => null }));

const readyStatus = { llm: { state: "ready" }, stt: { state: "ready" } };

function setUrl(search: string) {
	window.history.replaceState(null, "", `/chat${search}`);
}

describe("ChatSection", () => {
	beforeEach(() => {
		setUrl("");
	});
	afterEach(() => {
		setUrl("");
		vi.useRealTimers();
	});

	it("shows the draft-not-found error section when the draft can't be loaded", async () => {
		setUrl("?id=missing");
		mockInvoke({
			get_runtime_status: () => readyStatus,
			get_idea: () => {
				throw new Error("idea not found");
			}
		});
		render(<ChatSection draftId="missing" conversationEndCallbacks={() => {}} />);
		expect(await screen.findByText("Draft idea not found")).toBeInTheDocument();
		expect(screen.queryByText(/__DRAFT_NOT_FOUND__/)).not.toBeInTheDocument();
	});

	it("does not write a freshly loaded draft straight back to the database", async () => {
		vi.useFakeTimers({ shouldAdvanceTime: true });
		setUrl("?id=draft-1");
		mockInvoke({
			get_runtime_status: () => readyStatus,
			get_idea: () => ({
				id: "draft-1",
				transcript: [
					{ role: "assistant", content: "q1" },
					{ role: "user", content: "a1" },
					{ role: "assistant", content: "q2" },
					{ role: "user", content: "a2" },
					{ role: "assistant", content: "q3" }
				]
			}),
			update_idea: () => ({ id: "draft-1" })
		});
		render(<ChatSection draftId="draft-1" conversationEndCallbacks={() => {}} />);
		expect(await screen.findByText("q3")).toBeInTheDocument();
		await act(() => vi.advanceTimersByTimeAsync(1000));
		expect(vi.mocked(invoke).mock.calls.filter(([c]) => c === "update_idea")).toHaveLength(0);
	});

	describe("text composer", () => {
		async function sendTyped(text: string) {
			const user = userEvent.setup();
			await user.click(screen.getByText("Not in a place to talk out-loud?"));
			const box = screen.getByLabelText("Type your response");
			await user.type(box, text);
			await user.keyboard("{Enter}");
			return box;
		}

		it("clears the composer once the coach answered", async () => {
			mockInvoke({
				generate_response: () => ({ response: "tell me more" })
			});
			render(<ChatSection conversationEndCallbacks={() => {}} />);
			const box = await sendTyped("my idea");
			expect(await screen.findByText("tell me more")).toBeInTheDocument();
			await waitFor(() => expect(box).toHaveValue(""));
		});

		it("keeps the typed text when the message was flagged by moderation", async () => {
			mockInvoke({
				generate_response: () => {
					throw "HttpError 469: Inappropriate input";
				}
			});
			render(<ChatSection conversationEndCallbacks={() => {}} />);
			const box = await sendTyped("something flagged");
			expect(await screen.findByText(/flagged as inappropriate/)).toBeInTheDocument();
			expect(box).toHaveValue("something flagged");
		});

		it("keeps the text after an AI failure and resends it without duplicating it", async () => {
			let attempts = 0;
			mockInvoke({
				generate_response: () => {
					attempts++;
					// the first send fails twice (call + its one retry)
					if (attempts <= 2) throw "endpoint down";
					return { response: "got it" };
				}
			});
			render(<ChatSection conversationEndCallbacks={() => {}} />);
			const box = await sendTyped("my idea");
			expect(await screen.findByRole("alert")).toHaveTextContent("endpoint down");
			expect(box).toHaveValue("my idea");

			await userEvent.setup().click(screen.getByRole("button", { name: "Send message" }));
			expect(await screen.findByText("got it")).toBeInTheDocument();
			const lastCall = vi
				.mocked(invoke)
				.mock.calls.filter(([c]) => c === "generate_response")
				.at(-1)!;
			const sent = (lastCall[1] as { messages: { role: string; content: string }[] })
				.messages;
			expect(sent.filter((m) => m.content === "my idea")).toHaveLength(1);
		});
	});
});
