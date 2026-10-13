import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

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

	it("a failed status request says so instead of faking a zero streak", async () => {
		mockInvoke({
			get_daily_status: () => {
				throw new Error("db gone");
			}
		});
		render(<DailyStreakSection />);
		expect(await screen.findByText("Couldn’t load your streak.")).toBeInTheDocument();
		expect(screen.queryByText("0 days")).not.toBeInTheDocument();
		expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
	});

	it("an untouched day with a live streak names what keeps it", async () => {
		mockInvoke({
			get_daily_status: () => ({
				streak: 4,
				log_id: null,
				intent_idea_id: null,
				survey_id: null,
				is_completed: false
			})
		});
		render(<DailyStreakSection />);
		expect(
			await screen.findByText(/Do today’s daily intent to keep your 4-day streak/)
		).toBeInTheDocument();
	});

	it("a submitted log counts for today even before the intent starts", async () => {
		mockInvoke({
			get_daily_status: () => ({
				streak: 2,
				log_id: "log-1",
				intent_idea_id: null,
				survey_id: null,
				is_completed: false
			})
		});
		render(<DailyStreakSection />);
		expect(await screen.findByText("Today counts!")).toBeInTheDocument();
		expect(
			screen.getByRole("link", { name: "Think out loud about your day too" })
		).toHaveAttribute("href", "/chat?dailyIntent=true");
	});
});
