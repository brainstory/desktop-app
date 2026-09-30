import { describe, expect, it, vi } from "vitest";
import { render, screen, act } from "@testing-library/react";
import { useState } from "react";

import { Snackbar } from "./Snackbar";

describe("Snackbar", () => {
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
		function Host() {
			const [, setTick] = useState(0);
			// re-render after 2s (past the midway point of the countdown)
			setTimeout(() => setTick((t) => t + 1), 2000);
			return <Snackbar isSuccess={false} message="first" onClose={onClose} />;
		}
		render(<Host />);
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
