import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { TailwindComposedTabs } from "./TailwindTabs";

describe("TailwindComposedTabs", () => {
	const data = [
		{ label: "Summary", content: <p>summary body</p> },
		{ label: "Transcript", content: <p>transcript body</p> }
	];

	it("renders the active tab's panel", () => {
		render(
			<TailwindComposedTabs data={data} activeTab={0} tabParams={["summary", "transcript"]} />
		);
		expect(screen.getByRole("tabpanel")).toHaveTextContent("summary body");
	});

	it("clamps an out-of-range activeTab to the last real panel", () => {
		// a ?tab=feedback deep link on a page without a feedback tab
		render(
			<TailwindComposedTabs data={data} activeTab={2} tabParams={["summary", "transcript"]} />
		);
		expect(screen.getByRole("tabpanel")).toHaveTextContent("transcript body");
		expect(screen.getByRole("tab", { selected: true })).toHaveTextContent("Transcript");
	});

	describe("with more than two tabs", () => {
		// the clamp once counted TailwindTabs' own two children (tab list +
		// panels) instead of the tabs, so a third tab could never show
		const three = [...data, { label: "AI Models", content: <p>ai models body</p> }];
		const params = ["summary", "transcript", "aiModels"];

		it("deep-links to the third tab", () => {
			render(<TailwindComposedTabs data={three} activeTab={2} tabParams={params} />);
			expect(screen.getByRole("tabpanel")).toHaveTextContent("ai models body");
			expect(screen.getByRole("tab", { selected: true })).toHaveTextContent("AI Models");
		});

		it("shows the third tab's panel when it is clicked", async () => {
			const user = userEvent.setup();
			render(<TailwindComposedTabs data={three} activeTab={0} tabParams={params} />);
			await user.click(screen.getByRole("tab", { name: "AI Models" }));
			expect(screen.getByRole("tabpanel")).toHaveTextContent("ai models body");
		});

		it("arrow keys reach the third tab and wrap after it", async () => {
			const user = userEvent.setup();
			render(<TailwindComposedTabs data={three} activeTab={1} tabParams={params} />);
			screen.getByRole("tab", { name: "Transcript" }).focus();
			await user.keyboard("{ArrowRight}");
			expect(screen.getByRole("tabpanel")).toHaveTextContent("ai models body");
			await user.keyboard("{ArrowRight}");
			expect(screen.getByRole("tabpanel")).toHaveTextContent("summary body");
		});
	});

	it("arrow keys move between tabs", async () => {
		const user = userEvent.setup();
		render(
			<TailwindComposedTabs data={data} activeTab={0} tabParams={["summary", "transcript"]} />
		);
		const first = screen.getByRole("tab", { name: "Summary" });
		first.focus();
		await user.keyboard("{ArrowRight}");
		expect(screen.getByRole("tabpanel")).toHaveTextContent("transcript body");
		expect(screen.getByRole("tab", { selected: true })).toHaveTextContent("Transcript");
	});

	it("updates the ?tab= query param when a tab is selected", async () => {
		const user = userEvent.setup();
		window.history.replaceState(null, "", "/idea?id=x");
		render(
			<TailwindComposedTabs data={data} activeTab={0} tabParams={["summary", "transcript"]} />
		);
		await user.click(screen.getByRole("tab", { name: "Transcript" }));
		expect(new URLSearchParams(window.location.search).get("tab")).toBe("transcript");
	});

	it("disabled tabs are not clickable", async () => {
		const user = userEvent.setup();
		render(
			<TailwindComposedTabs
				data={[
					{ label: "Summary", content: <p>summary body</p> },
					{ label: "Feedback", content: <p>never shown</p>, disabled: true }
				]}
				activeTab={0}
				tabParams={["summary", "feedback"]}
			/>
		);
		await user.click(screen.getByRole("tab", { name: "Feedback" }));
		expect(screen.getByRole("tabpanel")).toHaveTextContent("summary body");
	});
});
