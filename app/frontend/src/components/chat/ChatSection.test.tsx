import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
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
});
