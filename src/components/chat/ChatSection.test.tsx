import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

import { cleanStores } from "nanostores";
import { mockInvoke } from "@src/test/mock-tauri";
import { $aiStatus } from "@components/global/aiStatusStore";
import { StrictMode } from "react";
import { ChatSection } from "./ChatSection";

// Rive needs a canvas + WASM runtime that jsdom doesn't have
vi.mock("@components/global/RivePencil", () => ({ default: () => null }));

const readyStatus = { llm: { state: "ready" }, stt: { state: "ready" } };

/** mockInvoke with a ready AI runtime unless the test overrides it */
function mockChat(handlers: Record<string, (args: unknown) => unknown>) {
	mockInvoke({ get_runtime_status: () => readyStatus, ...handlers });
}

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
		// unmount first: a still-mounted consumer would re-seed the store
		// right after cleanStores resets it
		cleanup();
		cleanStores($aiStatus);
	});

	it("shows the draft-not-found error section when the draft can't be loaded", async () => {
		setUrl("?id=missing");
		mockChat({
			get_idea: () => {
				throw new Error("idea not found");
			}
		});
		render(<ChatSection draftId="missing" conversationEndCallbacks={() => {}} />);
		expect(await screen.findByText("Draft idea not found")).toBeInTheDocument();
		expect(screen.queryByText(/__DRAFT_NOT_FOUND__/)).not.toBeInTheDocument();
	});

	it("shows 'Idea not found' when the idea to give feedback on is gone", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		mockChat({
			get_idea: () => {
				throw new Error("idea not found");
			}
		});
		render(
			<ChatSection
				chatType="feedback"
				parentIdParam="gone"
				conversationEndCallbacks={() => {}}
			/>
		);
		expect(await screen.findByText("Idea not found")).toBeInTheDocument();
		expect(screen.queryByText(/Shared idea/)).not.toBeInTheDocument();
		vi.mocked(console.error).mockRestore();
	});

	it("does not write a freshly loaded draft straight back to the database", async () => {
		vi.useFakeTimers({ shouldAdvanceTime: true });
		setUrl("?id=draft-1");
		mockChat({
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
			mockChat({
				generate_response: () => ({ response: "tell me more" })
			});
			render(<ChatSection conversationEndCallbacks={() => {}} />);
			const box = await sendTyped("my idea");
			expect(await screen.findByText("tell me more")).toBeInTheDocument();
			await waitFor(() => expect(box).toHaveValue(""));
		});

		it("keeps the typed text when the message was flagged by moderation", async () => {
			mockChat({
				generate_response: () => {
					throw "moderation: the AI provider flagged this message";
				}
			});
			render(<ChatSection conversationEndCallbacks={() => {}} />);
			const box = await sendTyped("something flagged");
			expect(await screen.findByText(/flagged as inappropriate/)).toBeInTheDocument();
			expect(box).toHaveValue("something flagged");
		});

		it("locks the composer while the coach is answering (no duplicate sends)", async () => {
			let answer!: (v: unknown) => void;
			mockChat({
				generate_response: () => new Promise((res) => (answer = res))
			});
			render(<ChatSection conversationEndCallbacks={() => {}} />);
			const box = await sendTyped("first");
			const send = screen.getByRole("button", { name: "Send message" });
			expect(send).toHaveAttribute("aria-disabled", "true");
			expect(box).toHaveAttribute("readonly");
			// focus stays in the composer (no disabled element drops it)
			expect(box).toHaveFocus();

			const user = userEvent.setup();
			await user.keyboard("{Enter}");
			await user.click(send);
			expect(
				vi.mocked(invoke).mock.calls.filter(([c]) => c === "generate_response")
			).toHaveLength(1);

			await act(async () => {
				answer({ response: "next question" });
				await Promise.resolve();
			});
			expect(await screen.findByText("next question")).toBeInTheDocument();
			expect(box).not.toHaveAttribute("readonly");
		});

		it("keeps the text after an AI failure and resends it without duplicating it", async () => {
			let attempts = 0;
			mockChat({
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

	describe("model status", () => {
		it("asks to download a model when none is installed (not 'loading' forever)", async () => {
			mockInvoke({
				get_runtime_status: () => ({ llm: { state: "missing" }, stt: { state: "ready" } })
			});
			render(<ChatSection conversationEndCallbacks={() => {}} />);
			const link = await screen.findByRole("link", { name: "Download a model in Settings" });
			expect(link).toHaveAttribute("href", "/profile?tab=aiModels");
			expect(screen.queryByText(/Loading the AI model/)).not.toBeInTheDocument();
		});

		it("says nothing before the runtime status is known", () => {
			mockInvoke({ get_runtime_status: () => new Promise(() => {}) });
			render(<ChatSection conversationEndCallbacks={() => {}} />);
			expect(screen.queryByText(/Loading the AI model/)).not.toBeInTheDocument();
			expect(screen.queryByText(/Download a model/)).not.toBeInTheDocument();
		});

		it("shows the loading notice while the model loads", async () => {
			mockInvoke({
				get_runtime_status: () => ({ llm: { state: "loading" }, stt: { state: "ready" } })
			});
			render(<ChatSection conversationEndCallbacks={() => {}} />);
			expect(await screen.findByText(/Loading the AI model/)).toBeInTheDocument();
		});

		it("locks the mic and Send (not typing) while the model loads", async () => {
			mockInvoke({
				get_runtime_status: () => ({ llm: { state: "loading" }, stt: { state: "ready" } })
			});
			render(<ChatSection conversationEndCallbacks={() => {}} />);
			await screen.findByText(/Loading the AI model/);
			const mic = screen.getByRole("button", { name: "Start recording" });
			expect(mic).toHaveAttribute("aria-disabled", "true");
			expect(screen.getByText("Loading model…")).toBeInTheDocument();

			const user = userEvent.setup();
			await user.click(mic);
			expect(
				vi.mocked(invoke).mock.calls.filter(([c]) => c === "start_voice_capture")
			).toHaveLength(0);

			await user.click(screen.getByText("Not in a place to talk out-loud?"));
			const box = screen.getByLabelText("Type your response");
			await user.type(box, "typed while loading");
			expect(box).toHaveValue("typed while loading");
			const send = screen.getByRole("button", { name: "Send message" });
			expect(send).toHaveAttribute("aria-disabled", "true");
			await user.keyboard("{Enter}");
			await user.click(send);
			expect(
				vi.mocked(invoke).mock.calls.filter(([c]) => c === "generate_response")
			).toHaveLength(0);
			expect(box).toHaveValue("typed while loading");
		});

		it("explains a failed model load", async () => {
			mockInvoke({
				get_runtime_status: () => ({
					llm: { state: "error", error: "bad gguf" },
					stt: { state: "ready" }
				})
			});
			render(<ChatSection conversationEndCallbacks={() => {}} />);
			expect(await screen.findByText(/failed to load: bad gguf/)).toBeInTheDocument();
		});
	});

	it("creates the idea exactly once at the minimum length, even under StrictMode", async () => {
		mockChat({
			create_idea: () => ({ id: "idea-1" }),
			update_idea: () => ({ id: "idea-1" }),
			generate_response: () => ({ response: "and then?" })
		});
		render(
			<StrictMode>
				<ChatSection chatType="daily_intent" conversationEndCallbacks={() => {}} />
			</StrictMode>
		);
		const user = userEvent.setup();
		await user.click(screen.getByText("Not in a place to talk out-loud?"));
		await user.type(screen.getByLabelText("Type your response"), "a calm day{Enter}");
		expect(await screen.findByText("and then?")).toBeInTheDocument();
		await waitFor(() =>
			expect(new URLSearchParams(window.location.search).get("id")).toBe("idea-1")
		);
		expect(vi.mocked(invoke).mock.calls.filter(([c]) => c === "create_idea")).toHaveLength(1);
	});

	it("does not re-fetch (or lose the chat over) the idea it just created", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		mockChat({
			create_idea: () => ({ id: "idea-1" }),
			update_idea: () => ({ id: "idea-1" }),
			// a transient read failure right after create
			get_idea: () => {
				throw new Error("database is locked");
			},
			generate_response: () => ({ response: "and then?" })
		});
		render(<ChatSection chatType="daily_intent" conversationEndCallbacks={() => {}} />);
		const user = userEvent.setup();
		await user.click(screen.getByText("Not in a place to talk out-loud?"));
		await user.type(screen.getByLabelText("Type your response"), "a calm day{Enter}");
		await waitFor(() =>
			expect(new URLSearchParams(window.location.search).get("id")).toBe("idea-1")
		);
		expect(await screen.findByText("and then?")).toBeInTheDocument();
		expect(screen.queryByText("Draft idea not found")).not.toBeInTheDocument();
		expect(vi.mocked(invoke).mock.calls.filter(([c]) => c === "get_idea")).toHaveLength(0);
		vi.mocked(console.error).mockRestore();
	});

	it("headlines a failed session save as a save error, not an AI error", async () => {
		mockChat({
			create_idea: () => {
				throw "database is locked";
			},
			generate_response: () => ({ response: "and then?" })
		});
		render(<ChatSection chatType="daily_intent" conversationEndCallbacks={() => {}} />);
		const user = userEvent.setup();
		await user.click(screen.getByText("Not in a place to talk out-loud?"));
		await user.type(screen.getByLabelText("Type your response"), "a calm day{Enter}");
		const alert = await screen.findByRole("alert");
		expect(alert).toHaveTextContent("Not saved yet: Could not save this session");
		expect(alert).not.toHaveTextContent(/AI couldn/);
	});

	describe("feedback on an idea", () => {
		async function sendFeedback(parent: Record<string, unknown>) {
			mockChat({
				get_idea: () => parent,
				generate_response: () => ({ response: "what stands out?" })
			});
			render(
				<ChatSection
					chatType="feedback"
					parentIdParam="parent-1"
					conversationEndCallbacks={() => {}}
				/>
			);
			// the feedback layout replaces the default one once the parent loads
			await screen.findByText("Let's take this idea even further!");
			const user = userEvent.setup();
			await user.click(screen.getByText("Not in a place to talk out-loud?"));
			await user.type(screen.getByLabelText("Type your response"), "I like it{Enter}");
			await screen.findByText("what stands out?");
			const call = vi.mocked(invoke).mock.calls.find(([c]) => c === "generate_response")!;
			return call[1] as {
				reactTo: string | null;
				reactToAuthor: string | null;
				reactToIsCurrentUser: boolean;
			};
		}

		it("tells the model an imported idea was written by its author, not the user", async () => {
			const args = await sendFeedback({
				id: "parent-1",
				title: "Ada's idea",
				result: "# Ada's idea",
				creator_name: "Ada"
			});
			expect(args.reactTo).toBe("# Ada's idea");
			expect(args.reactToAuthor).toBe("Ada");
			expect(args.reactToIsCurrentUser).toBe(false);
		});

		it("treats an idea without a creator as the user's own", async () => {
			const args = await sendFeedback({
				id: "parent-1",
				title: "My idea",
				result: "# My idea"
			});
			expect(args.reactToAuthor).toBeNull();
			expect(args.reactToIsCurrentUser).toBe(true);
		});
	});
});
