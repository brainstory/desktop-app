import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import IdeaFeedbackCard from "./IdeaFeedbackCard";
import EmojiList from "./EmojiList";
import type { FeedbackComment } from "@src/types";

// `labels` mimics old stored data: the LLM's emoji must not be shown
const comment = {
	commentId: "c1",
	feedbackText: "Love this part",
	labels: [{ name: "like", emoji: "👍" }],
	creatorName: "Ada"
} as unknown as FeedbackComment;

describe("IdeaFeedbackCard", () => {
	it("never nests a button inside its card button", () => {
		const { container } = render(
			<IdeaFeedbackCard
				feedback={{ ...comment, ideaId: "i1", hid: 1 }}
				focusSection={() => {}}
			/>
		);
		expect(container.querySelector("button button")).toBeNull();
		expect(screen.getAllByRole("button")).toHaveLength(1);
	});

	it("shows who wrote the comment but no LLM label emoji", () => {
		const { container } = render(
			<IdeaFeedbackCard
				feedback={{ ...comment, ideaId: "i1", hid: 1 }}
				focusSection={() => {}}
			/>
		);
		expect(screen.getByText("Ada")).toBeInTheDocument();
		expect(container.textContent).not.toContain("👍");
	});

	it("focuses the comment's section when the card is clicked", async () => {
		const user = userEvent.setup();
		const focusSection = vi.fn();
		render(
			<IdeaFeedbackCard
				feedback={{ ...comment, ideaId: "i1", hid: 1 }}
				focusSection={focusSection}
			/>
		);
		await user.click(screen.getByRole("button"));
		expect(focusSection).toHaveBeenCalledTimes(1);
	});
});

describe("EmojiList", () => {
	it("keeps reactions clickable as buttons when a handler is given", async () => {
		const user = userEvent.setup();
		const onReactionClick = vi.fn();
		render(<EmojiList reactions={[comment]} onReactionClick={onReactionClick} />);
		await user.click(screen.getByRole("button", { name: "Show feedback from Ada" }));
		expect(onReactionClick).toHaveBeenCalledWith(comment);
	});

	it("shows the commenter, not the LLM label emoji", () => {
		const { container } = render(
			<EmojiList reactions={[comment]} onReactionClick={() => {}} />
		);
		expect(container.textContent).not.toContain("👍");
	});

	it("highlights reactions with the pink accent ramp, not blue", () => {
		const { container } = render(
			<EmojiList reactions={[comment]} onReactionClick={() => {}} />
		);
		const chip = container.querySelector(".border.rounded-md");
		expect(chip).toHaveClass("border-accent-400", "bg-accent-50");
		expect(container.innerHTML).not.toMatch(/\bblue-\d/);
	});
});
