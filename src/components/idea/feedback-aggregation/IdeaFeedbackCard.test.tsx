import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import IdeaFeedbackCard from "./IdeaFeedbackCard";
import SectionCommenters from "./SectionCommenters";
import type { FeedbackComment } from "@src/types";

// `labels` mimics old stored data: the LLM's emoji must not be shown
const comment = {
	commentId: "c1",
	feedbackText: "Love this part",
	labels: [{ name: "like", emoji: "👍" }],
	creatorName: "Ada"
} as unknown as FeedbackComment;

function renderCard(props: Partial<Parameters<typeof IdeaFeedbackCard>[0]> = {}) {
	return render(
		<IdeaFeedbackCard
			feedback={{ ...comment, ideaId: "i1", hid: 1 }}
			focusSection={() => {}}
			myReactions={[]}
			onToggleReaction={() => {}}
			{...props}
		/>
	);
}

describe("IdeaFeedbackCard", () => {
	it("never nests a button inside its card button", () => {
		const { container } = renderCard({ myReactions: ["👍"] });
		expect(container.querySelector("button button")).toBeNull();
		// the card itself, plus the reaction chip and "React" beside it
		expect(screen.getAllByRole("button")).toHaveLength(3);
	});

	it("shows who wrote the comment but no LLM label emoji", () => {
		const { container } = renderCard();
		expect(screen.getByText("Ada")).toBeInTheDocument();
		expect(container.textContent).not.toContain("👍");
	});

	it("focuses the comment's section when the card is clicked", async () => {
		const user = userEvent.setup();
		const focusSection = vi.fn();
		renderCard({ focusSection });
		await user.click(screen.getByRole("button", { name: /Love this part/ }));
		expect(focusSection).toHaveBeenCalledTimes(1);
	});

	it("lets the author react to the comment", async () => {
		const user = userEvent.setup();
		const onToggleReaction = vi.fn();
		const focusSection = vi.fn();
		renderCard({ myReactions: ["❓"], onToggleReaction, focusSection });
		const bar = screen.getByRole("group", { name: "Reactions to comment from Ada" });
		const question = within(bar).getByRole("button", { name: "question ❓, 1" });
		expect(question).toHaveAttribute("aria-pressed", "true");

		await user.click(question);
		expect(onToggleReaction).toHaveBeenLastCalledWith("❓");
		await user.click(within(bar).getByRole("button", { name: "React" }));
		await user.click(within(bar).getByRole("button", { name: "React with suggestion 💡" }));
		expect(onToggleReaction).toHaveBeenLastCalledWith("💡");
		// reacting doesn't also activate the card
		expect(focusSection).not.toHaveBeenCalled();
	});
});

describe("SectionCommenters", () => {
	it("keeps commenters clickable as buttons when a handler is given", async () => {
		const user = userEvent.setup();
		const onCommentClick = vi.fn();
		render(<SectionCommenters comments={[comment]} onCommentClick={onCommentClick} />);
		await user.click(screen.getByRole("button", { name: "Show feedback from Ada" }));
		expect(onCommentClick).toHaveBeenCalledWith(comment);
	});

	it("shows the commenter, not the LLM label emoji", () => {
		const { container } = render(
			<SectionCommenters comments={[comment]} onCommentClick={() => {}} />
		);
		expect(container.textContent).not.toContain("👍");
	});

	it("highlights commenters with the pink accent ramp, not blue", () => {
		const { container } = render(
			<SectionCommenters comments={[comment]} onCommentClick={() => {}} />
		);
		const chip = container.querySelector(".border.rounded-md");
		expect(chip).toHaveClass("border-accent-400", "bg-accent-50");
		expect(container.innerHTML).not.toMatch(/\bblue-\d/);
	});
});
