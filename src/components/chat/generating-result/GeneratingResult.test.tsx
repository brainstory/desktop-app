import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";

import GeneratingResult from "./GeneratingResult";

describe("GeneratingResult", () => {
	afterEach(() => {
		vi.useRealTimers();
	});

	it("stays plain text through slow, bursty streaming until the stream completes", async () => {
		vi.useFakeTimers();
		const { rerender } = render(<GeneratingResult summary="# Title" isComplete={false} />);
		await act(() => vi.advanceTimersByTimeAsync(1000));
		rerender(<GeneratingResult summary={"# Title\n\nfirst"} isComplete={false} />);
		// a long pause between chunks is not the end of the stream
		await act(() => vi.advanceTimersByTimeAsync(2000));
		expect(screen.queryByRole("heading", { name: "Title" })).not.toBeInTheDocument();
		expect(screen.getByText(/# Title/)).toBeInTheDocument();

		rerender(<GeneratingResult summary={"# Title\n\nfirst"} isComplete />);
		expect(screen.getByRole("heading", { name: "Title" })).toBeInTheDocument();
	});
});
