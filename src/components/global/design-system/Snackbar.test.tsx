import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, act } from "@testing-library/react";
import { useState } from "react";

import { Snackbar, useSnackbar } from "./Snackbar";

/** Re-renders itself once, 2s in (past the midway point of the countdown). */
function RerenderingHost({ onClose }: { onClose: () => void }) {
	const [, setTick] = useState(0);
	setTimeout(() => setTick((t) => t + 1), 2000);
	return <Snackbar isSuccess={false} message="first" onClose={onClose} />;
}

// a failing assertion must not leak fake timers into the next test
afterEach(() => {
	vi.useRealTimers();
});

describe("Snackbar", () => {
	it("a re-render with a NEW onClose identity does not restart the countdown", () => {
		vi.useFakeTimers();
		const calls: string[] = [];
		const { rerender } = render(
			<Snackbar isSuccess={true} message="Saved" onClose={() => calls.push("first")} />
		);
		act(() => {
			vi.advanceTimersByTime(3000);
		});
		// parents pass inline arrows: every render is a new function
		rerender(
			<Snackbar isSuccess={true} message="Saved" onClose={() => calls.push("second")} />
		);
		act(() => {
			vi.advanceTimersByTime(2000); // 5s since mount, 2s since the re-render
		});
		// fired on the original schedule, through the latest handler
		expect(calls).toEqual(["second"]);
	});

	it("auto-dismisses after five seconds", () => {
		vi.useFakeTimers();
		const onClose = vi.fn();
		render(<Snackbar isSuccess={true} message="Saved" onClose={onClose} />);
		expect(onClose).not.toHaveBeenCalled();
		act(() => {
			vi.advanceTimersByTime(4999);
		});
		expect(onClose).not.toHaveBeenCalled();
		act(() => {
			vi.advanceTimersByTime(1);
		});
		expect(onClose).toHaveBeenCalledTimes(1);
		vi.useRealTimers();
	});

	it("a parent re-render does not restart the countdown", () => {
		vi.useFakeTimers();
		const onClose = vi.fn();
		render(<RerenderingHost onClose={onClose} />);
		act(() => {
			vi.advanceTimersByTime(2000); // parent re-render happens here
		});
		act(() => {
			vi.advanceTimersByTime(3000); // 5s total from the original mount
		});
		expect(onClose).toHaveBeenCalledTimes(1);
		vi.useRealTimers();
	});

	it("can be dismissed with the close button", async () => {
		const { user } = { user: (await import("@testing-library/user-event")).default.setup() };
		const onClose = vi.fn();
		render(<Snackbar isSuccess={true} message="hi" onClose={onClose} />);
		await user.click(screen.getByRole("button", { name: "Dismiss message" }));
		expect(onClose).toHaveBeenCalledTimes(1);
	});
});

describe("useSnackbar", () => {
	function Harness() {
		const { openSnackbar, snackbars } = useSnackbar();
		return (
			<>
				<button onClick={() => openSnackbar(true, "Saved it")}>ok</button>
				<button onClick={() => openSnackbar(false, "Broke it")}>fail</button>
				{snackbars}
			</>
		);
	}

	it("shows success and error messages independently until dismissed", async () => {
		const user = (await import("@testing-library/user-event")).default.setup();
		render(<Harness />);
		expect(screen.queryByRole("status")).not.toBeInTheDocument();

		await user.click(screen.getByRole("button", { name: "ok" }));
		await user.click(screen.getByRole("button", { name: "fail" }));
		expect(screen.getByText("Saved it")).toBeInTheDocument();
		expect(screen.getByText("Broke it")).toBeInTheDocument();

		await user.click(screen.getAllByRole("button", { name: "Dismiss message" })[0]!);
		expect(screen.queryByText("Saved it")).not.toBeInTheDocument();
		expect(screen.getByText("Broke it")).toBeInTheDocument();
	});
});
