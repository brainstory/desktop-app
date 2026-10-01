import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";

import { TextComposer } from "./RecordingWarnings";

function Composer({ onSend }: { onSend: () => void }) {
	const [value, setValue] = useState("");
	return <TextComposer value={value} onChange={setValue} onSend={onSend} />;
}

describe("TextComposer", () => {
	it("sends on Enter", async () => {
		const onSend = vi.fn();
		render(<Composer onSend={onSend} />);
		const user = userEvent.setup();
		await user.type(screen.getByLabelText("Type your response"), "hello{Enter}");
		expect(onSend).toHaveBeenCalledTimes(1);
	});

	it("inserts a newline on Shift+Enter instead of sending", async () => {
		const onSend = vi.fn();
		render(<Composer onSend={onSend} />);
		const user = userEvent.setup();
		const box = screen.getByLabelText("Type your response");
		await user.type(box, "line one{Shift>}{Enter}{/Shift}line two");
		expect(box).toHaveValue("line one\nline two");
		expect(onSend).not.toHaveBeenCalled();
	});

	it("does not send whitespace-only text", async () => {
		const onSend = vi.fn();
		render(<Composer onSend={onSend} />);
		const user = userEvent.setup();
		await user.type(screen.getByLabelText("Type your response"), "   {Enter}");
		expect(onSend).not.toHaveBeenCalled();
		expect(screen.getByRole("button", { name: "Send message" })).toHaveAttribute(
			"aria-disabled",
			"true"
		);
	});
});
