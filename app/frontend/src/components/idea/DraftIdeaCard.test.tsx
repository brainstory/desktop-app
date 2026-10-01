import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { mockInvoke } from "@src/test/mock-tauri";
import DraftIdeaCard from "./DraftIdeaCard";

afterEach(() => {
	vi.useRealTimers();
});

function deleteButton(): HTMLElement {
	return screen.getByRole("button", { name: /delete draft/i });
}

describe("DraftIdeaCard delete", () => {
	it("announces the armed state once, outside the labelled button", () => {
		vi.useFakeTimers();
		mockInvoke({});
		render(<DraftIdeaCard id="d1" draftSummary="half a thought" />);

		const status = screen.getByRole("status");
		expect(status).toBeEmptyDOMElement();

		fireEvent.click(deleteButton());
		expect(deleteButton()).toHaveAccessibleName("Confirm delete draft");
		expect(deleteButton()).toHaveTextContent("Really delete? (5s)");
		expect(status).toHaveTextContent("Click again to delete, resets in 5 seconds");
		// the live region is not inside the button (whose aria-label would
		// override it) and does not change on every countdown tick
		expect(deleteButton()).not.toContainElement(status);
		act(() => {
			vi.advanceTimersByTime(1000);
		});
		expect(deleteButton()).toHaveTextContent("Really delete? (4s)");
		expect(status).toHaveTextContent("Click again to delete, resets in 5 seconds");

		act(() => {
			vi.advanceTimersByTime(4000);
		});
		expect(deleteButton()).toHaveAccessibleName("Delete draft");
		expect(status).toBeEmptyDOMElement();
	});

	it("deletes on the second click and tells the parent", async () => {
		const onDeleted = vi.fn();
		mockInvoke({ delete_idea: () => null });
		render(<DraftIdeaCard id="d1" onDeleted={onDeleted} />);
		fireEvent.click(deleteButton());
		fireEvent.click(deleteButton());
		await waitFor(() => expect(onDeleted).toHaveBeenCalledWith("d1"));
	});

	it("surfaces a failed delete instead of only logging it", async () => {
		const onDeleted = vi.fn();
		vi.spyOn(console, "error").mockImplementation(() => {});
		mockInvoke({
			delete_idea: () => {
				throw new Error("db locked");
			}
		});
		render(<DraftIdeaCard id="d1" onDeleted={onDeleted} />);
		fireEvent.click(deleteButton());
		fireEvent.click(deleteButton());
		expect(await screen.findByText("Error: Could not delete this draft")).toBeInTheDocument();
		expect(onDeleted).not.toHaveBeenCalled();
	});
});

describe("DraftIdeaCard contrast", () => {
	it("draft summary text is stone-500 on white (4.8:1), not stone-400 (2.5:1)", () => {
		mockInvoke({});
		render(<DraftIdeaCard id="d1" draftSummary="half a thought" />);
		const summary = screen.getByText("half a thought");
		expect(summary).toHaveClass("text-stone-500");
		expect(summary).not.toHaveClass("text-stone-400");
	});
});
