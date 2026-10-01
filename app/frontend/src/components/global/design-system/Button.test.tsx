import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import Button from "./Button";
import LoadingAnimation from "@components/global/LoadingAnimation";
import EmojiList from "@components/idea/feedback-aggregation/EmojiList";

function classTokens(el: Element): string[] {
	return (el.getAttribute("class") ?? "").split(/\s+/).filter(Boolean);
}

describe("conditional classes (cn)", () => {
	it("Button emits no stray false/undefined class tokens", () => {
		render(<Button>Go</Button>);
		const tokens = classTokens(screen.getByRole("button", { name: "Go" }));
		expect(tokens).not.toContain("false");
		expect(tokens).not.toContain("undefined");
	});

	it("Button still applies its conditional classes", () => {
		render(
			<Button full disabled>
				Wide
			</Button>
		);
		expect(screen.getByRole("button", { name: "Wide" })).toHaveClass(
			"w-full",
			"opacity-50",
			"cursor-not-allowed"
		);
	});

	it("LoadingAnimation emits no stray false/undefined class tokens", () => {
		render(<LoadingAnimation />);
		const tokens = classTokens(screen.getByRole("status"));
		expect(tokens).not.toContain("false");
		expect(tokens).not.toContain("undefined");
	});

	it("EmojiList emits no stray false/undefined class tokens when unfocused", () => {
		const { container } = render(<EmojiList reactions={[]} onReactionClick={() => {}} />);
		const tokens = classTokens(container.firstElementChild!);
		expect(tokens).not.toContain("false");
		expect(tokens).not.toContain("undefined");
	});
});
