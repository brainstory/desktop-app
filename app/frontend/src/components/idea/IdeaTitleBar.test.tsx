import { describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { mockInvoke } from "@src/test/mock-tauri";
import { invoke } from "@tauri-apps/api/core";
import IdeaTitleBar from "./IdeaTitleBar";
import type { IdeaDetail } from "@src/types";

const idea: IdeaDetail = {
	id: "i1",
	title: "Original title",
	isOwnIdea: true
} as IdeaDetail;

function renderBar(overrides: Partial<IdeaDetail> = {}) {
	return render(<IdeaTitleBar idea={{ ...idea, ...overrides }} isOwnIdea={true} />);
}

async function startEditing(user: ReturnType<typeof userEvent.setup>) {
	await user.click(screen.getByRole("button", { name: "Rename idea" }));
	return screen.getByLabelText("Idea title");
}

describe("IdeaTitleBar rename", () => {
	it("commits a valid title through update_idea", async () => {
		const user = userEvent.setup();
		mockInvoke({ update_idea: () => ({ id: "i1" }) });
		renderBar();
		const input = await startEditing(user);
		await user.clear(input);
		await user.type(input, "Better title{Enter}");
		expect(screen.getByText("Better title")).toBeInTheDocument();
		await waitFor(() =>
			expect(
				vi
					.mocked(invoke)
					.mock.calls.some(
						([cmd, args]) =>
							cmd === "update_idea" &&
							(args as { title?: string }).title === "Better title"
					)
			).toBe(true)
		);
	});

	it("rejects an empty title with an error snackbar", async () => {
		const user = userEvent.setup();
		mockInvoke({ update_idea: () => ({ id: "i1" }) });
		renderBar();
		const input = await startEditing(user);
		await user.clear(input);
		await user.type(input, "   {Enter}");
		expect(await screen.findByText("Error: Title field is empty")).toBeInTheDocument();
		// the original title is untouched
		expect(screen.getByText("Original title")).toBeInTheDocument();
	});

	it("reverts the displayed title when the save fails", async () => {
		const user = userEvent.setup();
		mockInvoke({
			update_idea: () => {
				throw new Error("db locked");
			}
		});
		renderBar();
		const input = await startEditing(user);
		await user.clear(input);
		await user.type(input, "Doomed title{Enter}");
		expect(await screen.findByText("Error: Could not save the new title")).toBeInTheDocument();
		// optimistic update rolled back
		await waitFor(() => expect(screen.getByText("Original title")).toBeInTheDocument());
		expect(screen.queryByText("Doomed title")).not.toBeInTheDocument();
	});

	it("Escape discards the draft without saving", async () => {
		const user = userEvent.setup();
		mockInvoke({ update_idea: () => ({ id: "i1" }) });
		renderBar();
		const input = await startEditing(user);
		await user.clear(input);
		await user.type(input, "Discarded{Escape}");
		expect(screen.getByText("Original title")).toBeInTheDocument();
		expect(
			vi
				.mocked((await import("@tauri-apps/api/core")).invoke)
				.mock.calls.every(
					([cmd, args]) => !(cmd === "update_idea" && (args as { title?: string }).title)
				)
		).toBe(true);
	});
});

// Removing a focused input can fire `blur` (WebKit/Chromium webviews do)
// while React is committing the keydown's state change, i.e. with the
// previous render's onBlur closure. Simulate that by firing blur inside
// the same act() scope as the keydown, before the re-render lands.
function keyThenRemovalBlur(input: HTMLElement, key: string) {
	act(() => {
		fireEvent.keyDown(input, { key });
		fireEvent.blur(input);
	});
}

function updateIdeaTitles(): (string | undefined)[] {
	return vi
		.mocked(invoke)
		.mock.calls.filter(([cmd]) => cmd === "update_idea")
		.map(([, args]) => (args as { title?: string }).title);
}

describe("IdeaTitleBar edit session finishes exactly once", () => {
	it("Escape followed by a removal blur does not commit the draft", async () => {
		const user = userEvent.setup();
		mockInvoke({ update_idea: () => ({ id: "i1" }) });
		renderBar();
		const input = await startEditing(user);
		await user.clear(input);
		await user.type(input, "Discarded");
		keyThenRemovalBlur(input, "Escape");
		expect(screen.getByText("Original title")).toBeInTheDocument();
		await Promise.resolve();
		expect(updateIdeaTitles()).toEqual([]);
	});

	it("Enter followed by a removal blur renames once", async () => {
		const user = userEvent.setup();
		mockInvoke({ update_idea: () => ({ id: "i1" }) });
		renderBar();
		const input = await startEditing(user);
		await user.clear(input);
		await user.type(input, "Renamed");
		keyThenRemovalBlur(input, "Enter");
		expect(screen.getByText("Renamed")).toBeInTheDocument();
		await waitFor(() => expect(updateIdeaTitles()).toEqual(["Renamed"]));
	});

	it("the finish button saves once even though clicking it blurs the input", async () => {
		const user = userEvent.setup();
		mockInvoke({ update_idea: () => ({ id: "i1" }) });
		renderBar();
		const input = await startEditing(user);
		await user.clear(input);
		await user.type(input, "Clicked");
		await user.click(screen.getByRole("button", { name: "Finish editing" }));
		await waitFor(() => expect(updateIdeaTitles()).toEqual(["Clicked"]));
	});

	it("a new edit session after a finished one still saves", async () => {
		const user = userEvent.setup();
		mockInvoke({ update_idea: () => ({ id: "i1" }) });
		renderBar();
		let input = await startEditing(user);
		await user.type(input, "{Escape}");
		input = await startEditing(user);
		await user.clear(input);
		await user.type(input, "Second try{Enter}");
		await waitFor(() => expect(updateIdeaTitles()).toEqual(["Second try"]));
	});
});

describe("IdeaTitleBar delete", () => {
	it("shows a countdown and announces the armed state once", () => {
		vi.useFakeTimers();
		try {
			mockInvoke({});
			renderBar();
			const status = screen.getByRole("status");
			expect(status).toBeEmptyDOMElement();

			fireEvent.click(screen.getByRole("button", { name: "Delete" }));
			const armed = screen.getByRole("button", { name: /Really delete\?/ });
			expect(armed).toHaveTextContent("Really delete? (5s)");
			expect(status).toHaveTextContent("Click again to delete, resets in 5 seconds");
			expect(armed).not.toContainElement(status);

			act(() => {
				vi.advanceTimersByTime(2000);
			});
			expect(armed).toHaveTextContent("Really delete? (3s)");
			expect(status).toHaveTextContent("Click again to delete, resets in 5 seconds");

			act(() => {
				vi.advanceTimersByTime(3000);
			});
			expect(screen.getByRole("button", { name: "Delete" })).toBeInTheDocument();
			expect(status).toBeEmptyDOMElement();
		} finally {
			vi.useRealTimers();
		}
	});

	it("deletes on the second click", async () => {
		mockInvoke({ delete_idea: () => null });
		const assign = vi.fn();
		vi.stubGlobal("location", {
			...window.location,
			set href(v: string) {
				assign(v);
			}
		});
		try {
			renderBar();
			fireEvent.click(screen.getByRole("button", { name: "Delete" }));
			fireEvent.click(screen.getByRole("button", { name: /Really delete\?/ }));
			await waitFor(() => expect(assign).toHaveBeenCalledWith("/dashboard"));
		} finally {
			vi.unstubAllGlobals();
		}
	});
});
