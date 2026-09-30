import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
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
	return render(
		<IdeaTitleBar idea={{ ...idea, ...overrides } as IdeaDetail} isOwnIdea={true} />
	);
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
				vi.mocked(invoke).mock.calls.some(([cmd, args]) => cmd === "update_idea" && (args as { title?: string }).title === "Better title")
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
			vi.mocked((await import("@tauri-apps/api/core")).invoke).mock.calls.every(
				([cmd, args]) => !(cmd === "update_idea" && (args as { title?: string }).title)
			)
		).toBe(true);
	});
});
