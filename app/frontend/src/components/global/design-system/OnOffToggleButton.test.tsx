import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import OnOffToggleButton from "./OnOffToggleButton";

describe("OnOffToggleButton", () => {
	it("is controlled: it never flips without the parent updating `checked`", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		const { rerender } = render(<OnOffToggleButton checked={false} onToggle={onToggle} />);
		const sw = screen.getByRole("switch");
		expect(sw).toHaveAttribute("aria-checked", "false");

		// parent refuses the change (does not rerender with checked=true)
		await user.click(sw);
		expect(onToggle).toHaveBeenCalledWith(true);
		expect(sw).toHaveAttribute("aria-checked", "false");

		// parent accepts -> the switch follows
		rerender(<OnOffToggleButton checked={true} onToggle={onToggle} />);
		expect(sw).toHaveAttribute("aria-checked", "true");
	});

	it("toggles with Space and Enter like a native button", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		render(<OnOffToggleButton checked={false} onToggle={onToggle} />);
		const sw = screen.getByRole("switch");
		sw.focus();
		await user.keyboard(" ");
		await user.keyboard("{Enter}");
		expect(onToggle).toHaveBeenCalledTimes(2);
		expect(onToggle).toHaveBeenNthCalledWith(1, true);
	});

	it("does not fire when disabled", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		render(<OnOffToggleButton checked={false} onToggle={onToggle} disabled />);
		await user.click(screen.getByRole("switch"));
		expect(onToggle).not.toHaveBeenCalled();
		expect(screen.getByRole("switch")).toBeDisabled();
	});

	it("shows the on/off labels", () => {
		render(
			<OnOffToggleButton
				checked={true}
				onToggle={() => {}}
				checkedState="Yes"
				uncheckedState="No"
			/>
		);
		expect(screen.getByRole("switch")).toHaveTextContent("Yes");
	});
});
