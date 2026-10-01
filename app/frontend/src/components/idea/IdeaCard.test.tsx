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

	it("feedback cards open in the same window (no target=_blank in the desktop app)", () => {
		render(<IdeaCard id="f1" title="Some feedback" isFeedback shared />);
		const link = screen.getByRole("link", { name: /Some feedback/ });
		expect(link).toHaveAttribute("href", "/idea?id=f1");
		expect(link).not.toHaveAttribute("target");
	});
});
