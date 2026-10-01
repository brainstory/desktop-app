import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { CONVERSATION_STATE, type ConversationState } from "@src/const";
import { MicButton } from "./MicButton";

function renderMic(conversationState: ConversationState, onToggle = vi.fn()) {
	render(
		<MicButton
			isRecording={false}
			isDisabled={false}
			micStarting={false}
			conversationState={conversationState}
			elapsedSeconds={0}
			status="idle"
			onToggle={onToggle}
			onAnimationTrigger={() => {}}
		/>
	);
	return { onToggle, button: screen.getByRole("button", { name: "Start recording" }) };
}

describe("MicButton", () => {
	it("stays focusable but inert while waiting on the coach", async () => {
		const { onToggle, button } = renderMic(CONVERSATION_STATE.WaitingForCoach);
		// a disabled button loses focus (spec focus fixup); aria-disabled keeps it
		expect(button).not.toBeDisabled();
		expect(button).toHaveAttribute("aria-disabled", "true");
		button.focus();
		expect(button).toHaveFocus();
		await userEvent.setup().click(button);
		expect(onToggle).not.toHaveBeenCalled();
	});

	it("toggles when idle", async () => {
		const { onToggle, button } = renderMic(CONVERSATION_STATE.Idle);
		expect(button).toHaveAttribute("aria-disabled", "false");
		await userEvent.setup().click(button);
		expect(onToggle).toHaveBeenCalledTimes(1);
	});
});
