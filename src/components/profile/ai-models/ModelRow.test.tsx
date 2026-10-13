import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { ModelRow } from "./ModelRow";
import type { ModelStatus } from "@helpers/api/models";

const model: ModelStatus = {
	id: "m1",
	label: "MiniCPM5 2B",
	description: "A compact model",
	kind: "llm",
	sizeBytes: 1_500_000_000,
	downloaded: false,
	active: false,
	downloading: false
};

function renderRow(
	overrides: {
		model?: Partial<ModelStatus>;
		progress?: number;
		error?: string;
		pending?: "delete" | "activate";
	} = {}
) {
	const actions = {
		onDownload: vi.fn(),
		onCancel: vi.fn(),
		onDelete: vi.fn(),
		onActivate: vi.fn()
	};
	render(
		<ModelRow
			model={{ ...model, ...overrides.model }}
			progress={overrides.progress}
			error={overrides.error}
			pending={overrides.pending}
			freeBytes={null}
			{...actions}
		/>
	);
	return actions;
}

describe("ModelRow", () => {
	it("offers the download with its size and reports the click", async () => {
		const user = userEvent.setup();
		const actions = renderRow();
		await user.click(screen.getByRole("button", { name: "Download (1.5 GB)" }));
		expect(actions.onDownload).toHaveBeenCalledWith("m1");
	});

	it("a failed download reads as a retry, keeping the reason visible", async () => {
		const user = userEvent.setup();
		const actions = renderRow({ error: "The download stalled — try again." });
		expect(screen.getByRole("alert")).toHaveTextContent("The download stalled");
		await user.click(screen.getByRole("button", { name: "Retry" }));
		expect(actions.onDownload).toHaveBeenCalledWith("m1");
	});

	it("activates a downloaded model", async () => {
		const user = userEvent.setup();
		const actions = renderRow({ model: { downloaded: true } });
		await user.click(screen.getByRole("button", { name: "Use" }));
		expect(actions.onActivate).toHaveBeenCalledWith("m1");
	});

	it("marks the active model and drops the Use button", () => {
		renderRow({ model: { downloaded: true, active: true } });
		expect(screen.getByText("Active")).toBeInTheDocument();
		expect(screen.queryByRole("button", { name: "Use" })).not.toBeInTheDocument();
	});

	it("shows progress while downloading and cancels on request", async () => {
		const user = userEvent.setup();
		const actions = renderRow({ model: { downloading: true }, progress: 42.7 });
		const bar = screen.getByRole("progressbar", { name: "MiniCPM5 2B download progress" });
		expect(bar).toHaveAttribute("aria-valuenow", "42");
		expect(screen.getByText("42%")).toBeInTheDocument();
		await user.click(screen.getByRole("button", { name: "Cancel" }));
		expect(actions.onCancel).toHaveBeenCalledWith("m1");
	});

	it("deletes on the second click of the armed button", async () => {
		const user = userEvent.setup();
		const actions = renderRow({ model: { downloaded: true } });
		await user.click(screen.getByRole("button", { name: "Delete" }));
		await user.click(screen.getByRole("button", { name: /Really delete\?/ }));
		expect(actions.onDelete).toHaveBeenCalledWith("m1");
	});

	it("says that a slow delete is running instead of appearing dead", () => {
		renderRow({ model: { downloaded: true }, pending: "delete" });
		expect(screen.getByText("Deleting…")).toBeInTheDocument();
		// every action on the row is locked while it runs
		for (const button of screen.getAllByRole("button")) {
			expect(button).toBeDisabled();
		}
	});

	it("labels a slow activation", () => {
		renderRow({ model: { downloaded: true }, pending: "activate" });
		expect(screen.getByRole("button", { name: "Activating…" })).toBeDisabled();
	});
});
