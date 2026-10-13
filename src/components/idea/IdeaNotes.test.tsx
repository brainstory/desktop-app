import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";

import { mockInvoke } from "@src/test/mock-tauri";
import IdeaNotes from "./IdeaNotes";
import type { IdeaNote } from "@src/types";

const notes: IdeaNote[] = [
	{ id: "n1", text: "First thought", createdAt: "2026-01-02T10:00:00" },
	{ id: "n2", text: "Second thought", createdAt: "2026-01-03T10:00:00" }
];

describe("IdeaNotes", () => {
	it("renders nothing without notes", () => {
		const { container } = render(<IdeaNotes ideaId="i1" notes={[]} onChanged={() => {}} />);
		expect(container).toBeEmptyDOMElement();
	});

	it("lists the notes", () => {
		render(<IdeaNotes ideaId="i1" notes={notes} onChanged={() => {}} />);
		expect(screen.getByText("First thought")).toBeInTheDocument();
		expect(screen.getByText("Second thought")).toBeInTheDocument();
	});

	it("edits one note through update_idea_note", async () => {
		const user = userEvent.setup();
		mockInvoke({ update_idea_note: () => ({ id: "i1", noteId: "n2" }) });
		const onChanged = vi.fn();
		render(<IdeaNotes ideaId="i1" notes={notes} onChanged={onChanged} />);

		await user.click(screen.getAllByRole("button", { name: "Edit" })[1]!);
		const textarea = screen.getByLabelText("Note");
		expect(textarea).toHaveValue("Second thought");
		await user.clear(textarea);
		await user.type(textarea, "Second thought fixed");
		await user.click(screen.getByRole("button", { name: "Save changes" }));

		await waitFor(() => {
			const call = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "update_idea_note");
			expect(call?.[1]).toEqual({
				ideaId: "i1",
				noteId: "n2",
				note: "Second thought fixed"
			});
		});
		expect(onChanged).toHaveBeenCalled();
	});

	it("removes a note on the second click", async () => {
		const user = userEvent.setup();
		mockInvoke({ delete_idea_note: () => ({ id: "i1", noteId: "n1" }) });
		const onChanged = vi.fn();
		render(<IdeaNotes ideaId="i1" notes={notes} onChanged={onChanged} />);

		await user.click(screen.getAllByRole("button", { name: "Remove" })[0]!);
		await user.click(screen.getByRole("button", { name: /Remove\?/ }));

		await waitFor(() =>
			expect(vi.mocked(invoke)).toHaveBeenCalledWith("delete_idea_note", {
				ideaId: "i1",
				noteId: "n1"
			})
		);
		expect(onChanged).toHaveBeenCalled();
	});

	it("keeps the note visible and reports a failed remove", async () => {
		const user = userEvent.setup();
		mockInvoke({
			delete_idea_note: () => {
				throw new Error("db locked");
			}
		});
		const onChanged = vi.fn();
		render(<IdeaNotes ideaId="i1" notes={notes} onChanged={onChanged} />);

		await user.click(screen.getAllByRole("button", { name: "Remove" })[0]!);
		await user.click(screen.getByRole("button", { name: /Remove\?/ }));

		expect(await screen.findByRole("alert")).toHaveTextContent("db locked");
		expect(screen.getByText("First thought")).toBeInTheDocument();
		expect(onChanged).not.toHaveBeenCalled();
	});
});
