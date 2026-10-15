import { describe, expect, it, vi } from "vitest";
import { createRef } from "react";
import { render, screen } from "@testing-library/react";

import Button from "./Button";
import LoadingAnimation from "@components/global/LoadingAnimation";
import SectionCommenters from "@components/idea/feedback-aggregation/SectionCommenters";

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

	it("variants add their colour classes before the caller's classes", () => {
		render(
			<Button variant="pink" classes="mt-6">
				Save
			</Button>
		);
		const tokens = classTokens(screen.getByRole("button", { name: "Save" }));
		expect(tokens.slice(-4)).toEqual([
			"bg-accent-600",
			"hover:bg-accent-700",
			"text-white",
			"mt-6"
		]);
	});

	it("forwards a ref to the underlying button", () => {
		const ref = createRef<HTMLButtonElement>();
		render(<Button variant="transparent" ref={ref} sr="Open menu" icon="menu" />);
		expect(ref.current).toBe(screen.getByRole("button", { name: "Open menu" }));
	});

	it("renders a real anchor for a same-app path", () => {
		render(<Button href="/profile?tab=aiModels">Settings</Button>);
		const link = screen.getByRole("link", { name: "Settings" });
		expect(link).toHaveAttribute("href", "/profile?tab=aiModels");
	});

	it("degrades to a button and logs when the href is not navigable", () => {
		const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
		for (const href of ["javascript:alert(1)", "//evil.com", "/\\evil.com"]) {
			const { unmount } = render(<Button href={href}>Go</Button>);
			expect(screen.getByRole("button", { name: "Go" })).toBeInTheDocument();
			expect(screen.queryByRole("link")).not.toBeInTheDocument();
			unmount();
		}
		expect(
			errorSpy.mock.calls.filter(([m]) => String(m).includes("refusing unsafe href"))
		).length(3);
		errorSpy.mockRestore();
	});

	it("LoadingAnimation emits no stray false/undefined class tokens", () => {
		render(<LoadingAnimation />);
		const tokens = classTokens(screen.getByRole("status"));
		expect(tokens).not.toContain("false");
		expect(tokens).not.toContain("undefined");
	});

	it("SectionCommenters emits no stray false/undefined class tokens when unfocused", () => {
		const { container } = render(<SectionCommenters comments={[]} onCommentClick={() => {}} />);
		const tokens = classTokens(container.firstElementChild!);
		expect(tokens).not.toContain("false");
		expect(tokens).not.toContain("undefined");
	});
});
