import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import FinishedResultSection from "./FinishedResultSection";

describe("FinishedResultSection", () => {
	it("shows a failed save with a retry instead of the 'Hang tight' spinner", async () => {
		const onRetrySave = vi.fn();
		render(
			<FinishedResultSection
				ideaId="idea-1"
				result="# My summary"
				readyForFinish={false}
				isComplete
				saveError="disk full"
				onRetrySave={onRetrySave}
			/>
		);
		expect(screen.queryByText(/Hang tight/)).not.toBeInTheDocument();
		const alert = screen.getByRole("alert");
		expect(alert).toHaveTextContent("Couldn’t save your summary");
		expect(alert).toHaveTextContent("disk full");
		// the generated summary is kept
		expect(screen.getByRole("heading", { name: "My summary" })).toBeInTheDocument();

		await userEvent.setup().click(screen.getByRole("button", { name: "Try saving again" }));
		expect(onRetrySave).toHaveBeenCalledTimes(1);
	});

	it("shows the saving spinner while the save is pending", () => {
		render(
			<FinishedResultSection
				ideaId="idea-1"
				result="# My summary"
				readyForFinish={false}
				isComplete
			/>
		);
		expect(screen.getByText(/Hang tight/)).toBeInTheDocument();
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
	});
});
