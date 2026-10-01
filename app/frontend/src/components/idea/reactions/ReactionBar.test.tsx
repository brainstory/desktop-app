import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import ReactionBar, { summarizeReactions } from "./ReactionBar";

describe("summarizeReactions", () => {
	it("groups by emoji in the fixed display order, You first", () => {
		expect(
			summarizeReactions([
				{ emoji: "🚀", mine: false, from: "Bo" },
				{ emoji: "👍", mine: false, from: "Ana" },
				{ emoji: "👍", mine: true },
				{ emoji: "💡", mine: false, from: null }
			])
		).toEqual([
			{ emoji: "👍", name: "agree", count: 2, mine: true, people: ["You", "Ana"] },
			{ emoji: "💡", name: "suggestion", count: 1, mine: false, people: ["Someone"] },
			{ emoji: "🚀", name: "action", count: 1, mine: false, people: ["Bo"] }
		]);
	});
});

describe("ReactionBar", () => {
	it("shows a chip per emoji with its count and who reacted", () => {
		render(
			<ReactionBar
				label="Alpha"
				reactions={[
					{ emoji: "👍", mine: true },
					{ emoji: "👍", mine: false, from: "Ana" },
					{ emoji: "💡", mine: false, from: "Bo" }
				]}
				onToggle={() => {}}
			/>
		);
		const bar = screen.getByRole("group", { name: "Reactions to Alpha" });
		const agree = within(bar).getByRole("button", { name: "agree 👍, 2" });
		expect(agree).toHaveAttribute("aria-pressed", "true");
		expect(agree).toHaveAccessibleDescription("You, Ana");
		const suggestion = within(bar).getByRole("button", { name: "suggestion 💡, 1" });
		expect(suggestion).toHaveAttribute("aria-pressed", "false");
		expect(suggestion).toHaveAccessibleDescription("Bo");
	});

	it("a chip click toggles the user's own reaction with that emoji", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		render(
			<ReactionBar
				label="Alpha"
				reactions={[{ emoji: "💡", mine: false, from: "Bo" }]}
				onToggle={onToggle}
			/>
		);
		await user.click(screen.getByRole("button", { name: "suggestion 💡, 1" }));
		expect(onToggle).toHaveBeenCalledExactlyOnceWith("💡");
	});

	it("the picker offers the eight reactions and toggles the chosen one", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		render(
			<ReactionBar
				label="Alpha"
				reactions={[{ emoji: "📚", mine: true }]}
				onToggle={onToggle}
			/>
		);
		const react = screen.getByRole("button", { name: "React" });
		expect(react).toHaveAttribute("aria-expanded", "false");
		await user.click(react);
		expect(react).toHaveAttribute("aria-expanded", "true");

		const picker = screen.getByRole("group", { name: "Pick a reaction" });
		expect(within(picker).getAllByRole("button")).toHaveLength(8);
		expect(within(picker).getByRole("button", { name: "React with info 📚" })).toHaveAttribute(
			"aria-pressed",
			"true"
		);
		expect(within(picker).getByRole("button", { name: "React with error ⚠️" })).toHaveAttribute(
			"aria-pressed",
			"false"
		);

		await user.click(within(picker).getByRole("button", { name: "React with action 🚀" }));
		expect(onToggle).toHaveBeenCalledExactlyOnceWith("🚀");
		expect(screen.queryByRole("group", { name: "Pick a reaction" })).not.toBeInTheDocument();
		expect(react).toHaveFocus();
	});

	it("is keyboard operable: Enter opens, arrows move, Escape closes back to React", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		render(<ReactionBar label="Alpha" reactions={[]} onToggle={onToggle} />);
		const react = screen.getByRole("button", { name: "React" });

		await user.tab();
		expect(react).toHaveFocus();
		await user.keyboard("{Enter}");
		expect(screen.getByRole("button", { name: "React with agree 👍" })).toHaveFocus();
		await user.keyboard("{ArrowRight}");
		expect(screen.getByRole("button", { name: "React with disagree 👎" })).toHaveFocus();
		await user.keyboard("{ArrowLeft}{ArrowLeft}");
		expect(screen.getByRole("button", { name: "React with action 🚀" })).toHaveFocus();

		await user.keyboard("{Escape}");
		expect(screen.queryByRole("group", { name: "Pick a reaction" })).not.toBeInTheDocument();
		expect(react).toHaveFocus();
		expect(react).toHaveAttribute("aria-expanded", "false");
		expect(onToggle).not.toHaveBeenCalled();
	});

	it("closes on a click outside without toggling anything", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		render(
			<>
				<p>elsewhere</p>
				<ReactionBar label="Alpha" reactions={[]} onToggle={onToggle} />
			</>
		);
		await user.click(screen.getByRole("button", { name: "React" }));
		expect(screen.getByRole("group", { name: "Pick a reaction" })).toBeInTheDocument();
		await user.click(screen.getByText("elsewhere"));
		expect(screen.queryByRole("group", { name: "Pick a reaction" })).not.toBeInTheDocument();
		expect(onToggle).not.toHaveBeenCalled();
	});

	it("shows its note inside the picker", async () => {
		const user = userEvent.setup();
		render(
			<ReactionBar label="Alpha" reactions={[]} onToggle={() => {}} note="Exported too." />
		);
		expect(screen.queryByText("Exported too.")).not.toBeInTheDocument();
		await user.click(screen.getByRole("button", { name: "React" }));
		expect(
			within(screen.getByRole("group", { name: "Pick a reaction" })).getByText(
				"Exported too."
			)
		).toBeInTheDocument();
	});
});
