import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import IdeaSummary from "./IdeaSummary";

describe("IdeaSummary", () => {
	it("renders the summary markdown", () => {
		render(<IdeaSummary content={"# Heading\n\nBody text"} />);
		expect(screen.getByRole("heading", { name: "Heading" })).toBeInTheDocument();
		expect(screen.getByText("Body text")).toBeInTheDocument();
	});

	it("shows an empty state (not an endless spinner) for a loaded idea without a summary", () => {
		render(<IdeaSummary content={null} />);
		expect(screen.queryByText("Loading idea...")).not.toBeInTheDocument();
		expect(screen.queryByRole("status")).not.toBeInTheDocument();
		expect(screen.getByText("This idea has no summary yet.")).toBeInTheDocument();
	});

	it("shows the spinner only while explicitly loading", () => {
		render(<IdeaSummary content={null} isLoading />);
		expect(screen.getByRole("status")).toHaveTextContent("Loading idea...");
	});
});
