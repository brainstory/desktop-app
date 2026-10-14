import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import { AboutCard } from "./AboutCard";

// the version call bypasses the core-invoke mock (it reads the webview
// internals directly in jsdom)
vi.mock("@tauri-apps/api/app", () => ({
	getVersion: vi.fn(async () => "1.2.3")
}));

// the clipboard is stubbed: jsdom has none
const writeText = vi.fn(async () => {});
vi.stubGlobal("navigator", { clipboard: { writeText } });

describe("AboutCard", () => {
	beforeEach(() => {
		writeText.mockClear();
	});
	afterEach(() => {
		vi.mocked(invoke).mockClear();
	});

	it("shows the app version", async () => {
		mockInvoke({
			get_runtime_status: () => null
		});
		render(<AboutCard openSnackbar={vi.fn()} />);
		expect(await screen.findByText(/Brainstory 1\.2\.3/)).toBeInTheDocument();
	});

	it("collects diagnostics into the clipboard", async () => {
		const openSnackbar = vi.fn();
		mockInvoke({
			"plugin:app|version": () => "1.2.3",
			copy_diagnostics: () => "Brainstory 1.2.3 (macos)\nLLM: Ready gemma"
		});
		render(<AboutCard openSnackbar={openSnackbar} />);
		await screen.findByText(/Brainstory 1\.2\.3/);

		fireEvent.click(screen.getByRole("button", { name: "Copy diagnostics" }));
		await waitFor(() => expect(invoke).toHaveBeenCalledWith("copy_diagnostics"));
		await waitFor(() => expect(writeText).toHaveBeenCalled());
		// the raw report travels verbatim, the snackbar only confirms
		expect(writeText.mock.calls[0]![0]).toContain("LLM: Ready gemma");
		expect(openSnackbar).toHaveBeenCalledWith(true, "Diagnostics copied");
	});

	it("opens the folders through the backend commands", () => {
		mockInvoke({
			"plugin:app|version": () => "1.2.3",
			open_data_folder: () => null,
			open_log_folder: () => null
		});
		render(<AboutCard openSnackbar={vi.fn()} />);
		fireEvent.click(screen.getByRole("button", { name: "Open data folder" }));
		fireEvent.click(screen.getByRole("button", { name: "Open log folder" }));
		const called = vi.mocked(invoke).mock.calls.map(([cmd]) => cmd);
		expect(called).toContain("open_data_folder");
		expect(called).toContain("open_log_folder");
	});
});
