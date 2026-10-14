import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import LastAnswerActions from "./LastAnswerActions";

describe("LastAnswerActions keyboard", () => {
	it("Escape discards the edit, Cmd+Enter saves it", async () => {
		const onEdit = vi.fn();
		render(<LastAnswerActions lastAnswer="orginal" onEdit={onEdit} onRemove={() => {}} />);
		const user = userEvent.setup();
		await user.click(screen.getByText("Edit"));

		const box = screen.getByLabelText("Edit your last answer");
		await user.clear(box);
		await user.type(box, "corrected");

		// Cmd+Enter saves the corrected answer
		await user.keyboard("{Meta>}{Enter}{/Meta}");
		expect(onEdit).toHaveBeenCalledWith("corrected");
		expect(screen.queryByLabelText("Edit your last answer")).not.toBeInTheDocument();

		// Escape discards without saving
		await user.click(screen.getByText("Edit"));
		await user.type(screen.getByLabelText("Edit your last answer"), " more");
		await user.keyboard("{Escape}");
		expect(onEdit).toHaveBeenCalledTimes(1);
		expect(screen.queryByLabelText("Edit your last answer")).not.toBeInTheDocument();
	});
});
