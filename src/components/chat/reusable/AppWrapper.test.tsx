import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { mockedListen } from "@src/test/mock-tauri";
import type { EventCallback } from "@tauri-apps/api/event";

import { AppWrapper } from "./AppWrapper";

describe("AppWrapper", () => {
	afterEach(() => {
		mockedListen.mockReset();
		mockedListen.mockImplementation(async () => () => {});
	});

	it("explains a blocked quick capture instead of navigating silently", async () => {
		const handlers: EventCallback<unknown>[] = [];
		mockedListen.mockImplementation(async (_event, handler) => {
			handlers.push(handler);
			return () => {};
		});
		render(
			<AppWrapper>
				<p>page content</p>
			</AppWrapper>
		);
		expect(screen.getByText("page content")).toBeInTheDocument();

		await act(async () => {
			await vi.waitFor(() => expect(mockedListen).toHaveBeenCalled());
		});
		// the backend refused the shortcut: a recording is in progress
		act(() => {
			for (const handler of handlers) {
				handler({ event: "quick-capture-blocked", id: 0, payload: null });
			}
		});
		expect(
			await screen.findByText("Finish or stop the current recording first.")
		).toBeInTheDocument();
	});
});
