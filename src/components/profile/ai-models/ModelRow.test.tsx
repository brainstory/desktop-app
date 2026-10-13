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
		pausedPct?: number;
		rate?: { bytesPerSec: number; etaSecs: number | null };
		error?: string;
		pending?: "delete" | "activate";
	} = {}
) {
	const actions = {
		onDownload: vi.fn(),
		onCancel: vi.fn(),
		onPause: vi.fn(),
		onDelete: vi.fn(),
		onActivate: vi.fn()
	};
	render(
		<ModelRow
			model={{ ...model, ...overrides.model }}
			progress={overrides.progress}
			pausedPct={overrides.pausedPct}
			rate={overrides.rate}
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

	it("pauses a running download on request", async () => {
		const user = userEvent.setup();
		const actions = renderRow({ model: { downloading: true }, progress: 30 });
		await user.click(screen.getByRole("button", { name: "Pause" }));
		expect(actions.onPause).toHaveBeenCalledWith("m1");
	});

	it("shows speed and estimated time left while downloading", () => {
		renderRow({
			model: { downloading: true },
			progress: 30,
			rate: { bytesPerSec: 12_300_000, etaSecs: 95 }
		});
		expect(screen.getByText("12.3 MB/s · about 1m 35s left")).toBeInTheDocument();
	});

	it("shows a paused download with Resume in place of Pause and Cancel", async () => {
		const user = userEvent.setup();
		const actions = renderRow({ pausedPct: 42.7 });
		const bar = screen.getByRole("progressbar", { name: "MiniCPM5 2B download progress" });
		expect(bar).toHaveAttribute("aria-valuenow", "42");
		expect(screen.getByText("Paused — Resume continues where it stopped.")).toBeInTheDocument();
		expect(screen.queryByRole("button", { name: "Pause" })).not.toBeInTheDocument();
		expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
		await user.click(screen.getByRole("button", { name: "Resume" }));
		expect(actions.onDownload).toHaveBeenCalledWith("m1");
	});

	it("reads a pause from the model listing when the store has no pct", () => {
		// a page opened while the download sits paused: only the backend
		// listing knows about it
		renderRow({ model: { paused: true, progress: 55.2 } });
		expect(screen.getByRole("button", { name: "Resume" })).toBeInTheDocument();
		expect(screen.getByText("55%")).toBeInTheDocument();
	});

	it("discards a paused download on the second click", async () => {
		const user = userEvent.setup();
		const actions = renderRow({ pausedPct: 10 });
		await user.click(screen.getByRole("button", { name: "Discard" }));
		await user.click(screen.getByRole("button", { name: /Really discard\?/ }));
		expect(actions.onDelete).toHaveBeenCalledWith("m1");
	});

	it("keeps the live download ahead of a stale paused flag", () => {
		// resumed: the listing may still say paused until it refreshes
		renderRow({ model: { paused: true, progress: 55 }, progress: 61 });
		expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
		expect(screen.queryByRole("button", { name: "Resume" })).not.toBeInTheDocument();
	});
});
