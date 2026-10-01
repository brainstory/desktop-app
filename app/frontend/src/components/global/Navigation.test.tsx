import { describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderToString } from "react-dom/server";

import { mockViewport } from "@src/test/match-media";
import Navigation from "./Navigation";

// the updater banner talks to the Tauri updater plugin; not under test here
vi.mock("@ds/UpdaterBanner", () => ({ default: () => null }));

/** jsdom does not implement `inert`, so assert on the attribute itself. */
function isInert(el: HTMLElement): boolean {
	return el.closest("[inert]") !== null;
}

describe("Navigation", () => {
	it("keeps the always-visible desktop sidebar interactive", () => {
		mockViewport(1024);
		render(<Navigation />);
		const dashboard = screen.getByRole("link", { name: "Dashboard" });
		expect(isInert(dashboard)).toBe(false);
		expect(isInert(screen.getByRole("link", { name: "Settings" }))).toBe(false);
	});

	it("makes the closed mobile drawer inert and the open one interactive", async () => {
		mockViewport(500);
		const user = userEvent.setup();
		render(<Navigation />);
		const dashboard = screen.getByRole("link", { name: "Dashboard", hidden: true });
		expect(isInert(dashboard)).toBe(true);

		await user.click(screen.getByRole("button", { name: "Open Sidebar" }));
		expect(isInert(dashboard)).toBe(false);
		expect(screen.getByRole("button", { name: "Close Sidebar" })).toHaveFocus();
	});

	it("Escape closes the drawer and returns focus to the hamburger", async () => {
		mockViewport(500);
		const user = userEvent.setup();
		render(<Navigation />);
		const hamburger = screen.getByRole("button", { name: "Open Sidebar" });
		await user.click(hamburger);
		await user.keyboard("{Escape}");
		expect(hamburger).toHaveFocus();
		expect(isInert(screen.getByRole("link", { name: "Dashboard", hidden: true }))).toBe(true);
	});

	it("follows the viewport: growing to desktop width lifts inert", () => {
		const viewport = mockViewport(500);
		render(<Navigation />);
		const dashboard = screen.getByRole("link", { name: "Dashboard", hidden: true });
		expect(isInert(dashboard)).toBe(true);
		act(() => viewport.setWidth(1024));
		expect(isInert(dashboard)).toBe(false);
	});

	it("marks the current page only after mount (no SSR/hydration mismatch)", () => {
		window.history.pushState({}, "", "/dashboard");
		try {
			// the server render must not depend on window.location
			expect(renderToString(<Navigation />)).not.toContain("aria-current");
			render(<Navigation />);
			expect(screen.getByRole("link", { name: "Dashboard" })).toHaveAttribute(
				"aria-current",
				"page"
			);
			expect(screen.getByRole("link", { name: "Settings" })).not.toHaveAttribute(
				"aria-current"
			);
		} finally {
			window.history.pushState({}, "", "/");
		}
	});

	it("SSR renders the sidebar interactive so desktop works before hydration", () => {
		expect(renderToString(<Navigation />)).not.toContain("inert");
	});
});
