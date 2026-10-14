import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

import StickyLoadingSection from "./StickyLoadingSection";

// the cancel button and the loading animation are exercised elsewhere;
// here only the truncation notice matters
vi.mock("@components/global/LoadingAnimation", () => ({ default: () => null }));
vi.mock("@components/chat/reusable/ChatStateNotification", () => ({
	CancelGenerationButton: () => null
}));

describe("StickyLoadingSection", () => {
	it("says nothing about truncation when the whole conversation fit", () => {
		render(<StickyLoadingSection isFinishedGenerating={false} ideaId="i1" droppedTurns={0} />);
		expect(screen.queryByText(/only read the last part/)).not.toBeInTheDocument();
	});

	it("warns with the dropped turn count while generating and when finished", () => {
		const { rerender } = render(
			<StickyLoadingSection isFinishedGenerating={false} ideaId="i1" droppedTurns={14} />
		);
		expect(screen.getByText(/only read the last part/)).toBeInTheDocument();
		expect(screen.getByText(/14 earlier turns were left out/)).toBeInTheDocument();

		rerender(<StickyLoadingSection isFinishedGenerating ideaId="i1" droppedTurns={14} />);
		expect(screen.getByText(/14 earlier turns were left out/)).toBeInTheDocument();
		expect(screen.getByRole("button", { name: "Open your idea →" })).toBeInTheDocument();
	});
});
