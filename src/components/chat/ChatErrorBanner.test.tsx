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

	it("headlines AI errors as the AI not responding", () => {
		render(<ChatErrorBanner aiError="endpoint down" onDismiss={() => {}} />);
		expect(screen.getByRole("alert")).toHaveTextContent("Hmm, the AI couldn’t respond:");
	});

	it("headlines save errors as a save problem, without the AI settings link", () => {
		render(
			<ChatErrorBanner
				aiError="Autosave failed: database is locked"
				source="save"
				onDismiss={() => {}}
			/>
		);
		const alert = screen.getByRole("alert");
		expect(alert).toHaveTextContent("Not saved yet: Autosave failed: database is locked");
		expect(alert).not.toHaveTextContent(/AI couldn/);
		expect(screen.queryByRole("link", { name: "Open AI settings" })).not.toBeInTheDocument();
	});

	it("passes unknown errors through", () => {
		render(<ChatErrorBanner aiError="disk full" onDismiss={() => {}} />);
		expect(screen.getByRole("alert")).toHaveTextContent("disk full");
	});
});
