import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

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
});
