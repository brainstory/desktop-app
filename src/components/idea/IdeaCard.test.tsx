import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import IdeaCard from "./IdeaCard";
import type { IdeaListItem } from "@src/types";

const feedback = [
	{ id: "f1", summaryPreview: "" },
	{ id: "f2", summaryPreview: "" }
] as IdeaListItem[];

describe("IdeaCard", () => {
	it("is an article with one title link and a sibling feedback link (no nesting)", () => {
		const { container } = render(
			<IdeaCard
				id="i1"
				title="My idea"
				createdAt="2026-01-01T00:00:00Z"
				feedback={feedback}
			/>
		);
		const article = screen.getByRole("article");
		expect(article).toBeInTheDocument();

		const titleLink = screen.getByRole("link", { name: /My idea/ });
		expect(titleLink).toHaveAttribute("href", "/idea?id=i1");

		const feedbackLink = screen.getByRole("link", { name: "2 feedback items" });
		expect(feedbackLink).toHaveAttribute("href", "/idea?id=i1&tab=feedback");

		// no interactive element nested inside another
		expect(container.querySelector("a a, a button, button a, button button")).toBeNull();
		expect(titleLink).not.toContainElement(feedbackLink);
	});

	it("counts and stacks only finished feedback, never an unfinished draft", () => {
		const { container, rerender } = render(
			<IdeaCard
				id="i1"
				title="My idea"
				feedback={[
					{ id: "f1", summaryPreview: "", isDraft: false },
					{ id: "fd1", summaryPreview: "", isDraft: true }
				]}
			/>
		);
		expect(screen.getByRole("link", { name: "1 feedback item" })).toBeInTheDocument();
		expect(container.querySelectorAll('[aria-hidden="true"].pointer-events-none')).toHaveLength(
			1
		);

		// only a draft: no feedback link and no stack at all
		rerender(
			<IdeaCard
				id="i1"
				title="My idea"
				feedback={[{ id: "fd1", summaryPreview: "", isDraft: true }]}
			/>
		);
		expect(screen.queryByRole("link", { name: /feedback item/ })).not.toBeInTheDocument();
		expect(container.querySelectorAll('[aria-hidden="true"].pointer-events-none')).toHaveLength(
			0
		);
	});

	it("feedback cards open in the same window (no target=_blank in the desktop app)", () => {
		render(<IdeaCard id="f1" title="Some feedback" isFeedback shared />);
		const link = screen.getByRole("link", { name: /Some feedback/ });
		expect(link).toHaveAttribute("href", "/idea?id=f1");
		expect(link).not.toHaveAttribute("target");
	});

	it("the New badge is white on a dark-enough accent (>= 4.5:1, not pink-500)", () => {
		render(<IdeaCard id="i1" title="Unread" isUnread />);
		const badge = screen.getByText("New");
		expect(badge).toHaveClass("bg-accent-700", "text-white");
		expect(badge).not.toHaveClass("bg-pink-500");
	});
});
