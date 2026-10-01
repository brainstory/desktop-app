import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { ChatErrorBanner } from "./ChatErrorBanner";

describe("ChatErrorBanner", () => {
	it("shows the friendly description and action instead of the raw error", () => {
		render(
			<ChatErrorBanner
				aiError="Language model not downloaded yet. Open Settings > AI Models to download one."
				onDismiss={() => {}}
			/>
		);
		const alert = screen.getByRole("alert");
		expect(alert).toHaveTextContent("No language model is downloaded yet.");
		expect(alert).not.toHaveTextContent(/speech/i);
		expect(screen.getByRole("link", { name: "Open AI settings" })).toHaveAttribute(
			"href",
			"/profile?tab=aiModels"
		);
	});

	it("passes unknown errors through", () => {
		render(<ChatErrorBanner aiError="disk full" onDismiss={() => {}} />);
		expect(screen.getByRole("alert")).toHaveTextContent("disk full");
	});
});
