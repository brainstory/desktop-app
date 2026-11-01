import { describe, expect, it, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";

import { useApiAction } from "./useApiAction";
import { useSnackbar } from "@ds/Snackbar";

// the context is the only collaborator: a recording stand-in keeps the
// assertions about what the caller would see
const openSnackbar = vi.fn<(isSuccess: boolean, message: string) => void>();
vi.mock("@ds/Snackbar", () => ({
	useSnackbar: () => ({ openSnackbar })
}));

describe("useApiAction", () => {
	it("resolves with the result and shows the success copy", async () => {
		const { result } = renderHook(() => useApiAction());
		const outcome = await result.current.run(Promise.resolve("ok"), { success: "Saved" });
		expect(outcome).toEqual({ ok: true, value: "ok" });
		await waitFor(() => expect(openSnackbar).toHaveBeenCalledWith(true, "Saved"));
	});

	it("shows the normalized error and resolves null instead of rejecting", async () => {
		const { result } = renderHook(() => useApiAction());
		const outcome = await result.current.run(Promise.reject(new Error("disk full")));
		expect(outcome).toEqual({ ok: false });
		await waitFor(() =>
			expect(openSnackbar).toHaveBeenCalledWith(false, expect.stringContaining("disk full"))
		);
	});

	it("tracks a keyed pending flag through the flight", async () => {
		let release!: (v: string) => void;
		const gated = new Promise<string>((res) => (release = res));
		const { result } = renderHook(() => useApiAction());
		const flight = result.current.run(gated, { key: "row-1" });
		await waitFor(() => expect(result.current.isPending("row-1")).toBe(true));
		expect(result.current.isPending("row-2")).toBe(false);
		release("done");
		await flight;
		await waitFor(() => expect(result.current.isPending("row-1")).toBe(false));
	});

	it("a silent success shows nothing", async () => {
		openSnackbar.mockClear();
		const { result } = renderHook(() => useApiAction());
		await result.current.run(Promise.resolve(1));
		expect(openSnackbar).not.toHaveBeenCalled();
	});
});

// keep the mocked module honest: the real hook is what runs in the app
void useSnackbar;
