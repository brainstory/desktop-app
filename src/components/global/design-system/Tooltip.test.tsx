import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import Tooltip from "./Tooltip";
import { TailwindComposedTabs } from "./TailwindTabs";

describe("Tooltip", () => {
	it("describes the focusable trigger, not a wrapper div", () => {
		const { container } = render(
			<Tooltip text="Another question">
				<button type="button">Skip</button>
			</Tooltip>
		);
		const trigger = screen.getByRole("button", { name: "Skip" });
		expect(trigger).toHaveAccessibleDescription("Another question");
		expect(container.querySelectorAll("[aria-describedby]")).toHaveLength(1);
	});

	it("keeps a description the trigger already had", () => {
		render(
			<>
				<p id="extra">Extra hint</p>
				<Tooltip text="Tip">
					<button type="button" aria-describedby="extra">
						Go
					</button>
				</Tooltip>
			</>
		);
		expect(screen.getByRole("button", { name: "Go" })).toHaveAccessibleDescription(
			"Extra hint Tip"
		);
	});

	it("adds no description when there is no text", () => {
		render(
			<Tooltip>
				<button type="button">Plain</button>
			</Tooltip>
		);
		expect(screen.getByRole("button", { name: "Plain" })).not.toHaveAttribute(
			"aria-describedby"
		);
		expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
	});
});

describe("TailwindTab tooltip", () => {
	it("an enabled tab is described by its tooltip", () => {
		render(
			<TailwindComposedTabs
				data={[
					{ label: "Summary", content: <p>summary</p>, tooltipText: "The short version" }
				]}
			/>
		);
		expect(screen.getByRole("tab", { name: "Summary" })).toHaveAccessibleDescription(
			"The short version"
		);
	});

	it("a disabled tab is described by its visible explanation", () => {
		render(
			<TailwindComposedTabs
				data={[
					{ label: "Summary", content: <p>summary</p> },
					{
						label: "Feedback",
						content: <p>none</p>,
						disabled: true,
						tooltipText: "No feedback on this idea yet"
					}
				]}
			/>
		);
		expect(screen.getByRole("tab", { name: "Feedback" })).toHaveAccessibleDescription(
			"No feedback on this idea yet"
		);
	});
});
