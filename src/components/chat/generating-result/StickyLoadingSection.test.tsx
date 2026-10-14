import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

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

	it("offers retry and keep-draft when the generation stopped early", async () => {
		const onRetryGeneration = vi.fn();
		const onKeepDraft = vi.fn();
		render(
			<StickyLoadingSection
				isFinishedGenerating={false}
				ideaId="i1"
				generationStopped
				onRetryGeneration={onRetryGeneration}
				onKeepDraft={onKeepDraft}
			/>
		);
		expect(screen.getByRole("alert")).toHaveTextContent("Generation stopped early");
		await userEvent.click(screen.getByRole("button", { name: "Try again" }));
		expect(onRetryGeneration).toHaveBeenCalledTimes(1);
		await userEvent.click(screen.getByRole("button", { name: "Keep this draft" }));
		expect(onKeepDraft).toHaveBeenCalledTimes(1);
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
