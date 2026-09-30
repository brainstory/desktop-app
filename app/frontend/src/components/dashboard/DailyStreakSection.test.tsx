import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

import { mockInvoke } from "@src/test/mock-tauri";
import DailyStreakSection from "./DailyStreakSection";

describe("DailyStreakSection", () => {
	it("shows the streak count and the completed link to the idea page", async () => {
		mockInvoke({
			get_daily_status: () => ({
				streak: 3,
				log_id: null,
				intent_idea_id: "done-1",
				survey_id: null,
				is_completed: true
			})
		});
		render(<DailyStreakSection />);
		expect(await screen.findByText("3 days")).toBeInTheDocument();
		const link = screen.getByRole("link", { name: "See today’s idea" });
		expect(link).toHaveAttribute("href", "/idea?id=done-1");
	});

	it("an in-progress intent links back to its chat draft, not the idea page", async () => {
		mockInvoke({
			get_daily_status: () => ({
				streak: 0,
				log_id: null,
				intent_idea_id: "draft-1",
				survey_id: null,
				is_completed: false
			})
		});
		render(<DailyStreakSection />);
		const link = await screen.findByRole("link", { name: "Finish today’s daily intent" });
		expect(link).toHaveAttribute("href", "/chat?dailyIntent=true&id=draft-1");
	});

	it("with no intent, links to a fresh daily intent chat", async () => {
		mockInvoke({
			get_daily_status: () => ({
				streak: 0,
				log_id: null,
				intent_idea_id: null,
				survey_id: null,
				is_completed: false
			})
		});
		render(<DailyStreakSection />);
		const link = await screen.findByRole("link", { name: "Do today’s daily intent" });
		expect(link).toHaveAttribute("href", "/chat?dailyIntent=true");
	});

	it("renders the streak even when the status request fails", async () => {
		mockInvoke({
			get_daily_status: () => {
				throw new Error("db gone");
			}
		});
		render(<DailyStreakSection />);
		await waitFor(() => expect(screen.getByText("0 days")).toBeInTheDocument());
	});
});
