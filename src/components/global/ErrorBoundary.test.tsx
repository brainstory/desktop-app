import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import ErrorBoundary from "./ErrorBoundary";

function Boom(): never {
	throw new Error("kaboom");
}

describe("ErrorBoundary", () => {
	beforeEach(() => {
		// React logs the caught render error; the fallback is the point
		vi.spyOn(console, "error").mockImplementation(() => {});
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("renders its children untouched while nothing throws", () => {
		render(
			<ErrorBoundary>
				<p>all good</p>
			</ErrorBoundary>
		);
		expect(screen.getByText("all good")).toBeInTheDocument();
	});

	it("catches a render crash and offers a reload", async () => {
		const user = userEvent.setup();
		const onReload = vi.fn();
		render(
			<ErrorBoundary title="This page hit a snag" onReload={onReload}>
				<Boom />
			</ErrorBoundary>
		);
		expect(screen.getByText("This page hit a snag")).toBeInTheDocument();
		// the message is shown so a bug report can quote it
		expect(screen.getByText(/kaboom/)).toBeInTheDocument();
		expect(screen.getByText(/Your ideas are safe on this device/)).toBeInTheDocument();

		await user.click(screen.getByRole("button", { name: "Reload" }));
		expect(onReload).toHaveBeenCalledTimes(1);
	});

	it("falls back to a generic title when none was given", () => {
		render(
			<ErrorBoundary onReload={() => {}}>
				<Boom />
			</ErrorBoundary>
		);
		expect(screen.getByText("Something went wrong")).toBeInTheDocument();
	});
});
