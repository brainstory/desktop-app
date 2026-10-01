import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { mockInvoke } from "@src/test/mock-tauri";
import { invoke } from "@tauri-apps/api/core";
import DashboardSection from "./DashboardSection";
import { hasDoneGettingStarted, markGettingStartedDone } from "@helpers/storage";

const emptyStatus = { llm: { state: "ready" }, stt: { state: "ready" } };
const emptyModels = {
	llm: [
		{
			id: "m",
			label: "M",
			description: "",
			kind: "llm",
			sizeBytes: 1,
			downloaded: true,
			active: true,
			downloading: false
		}
	],
	stt: []
};
const emptyAiSettings = {
	llmMode: "local",
	llmModel: "m",
	sttModel: "s",
	sttEngine: "whisper",
	sttLanguage: "en-US",
	hfTokenSet: false,
	hfTokenHint: null,
	extLlmBaseUrl: "",
	extLlmApiKeySet: false,
	extLlmApiKeyHint: null,
	extLlmModel: "",
	extSttBaseUrl: "",
	extSttApiKeySet: false,
	extSttApiKeyHint: null,
	extSttModel: ""
};

function mockBase(extra: Record<string, (args: unknown) => unknown> = {}) {
	mockInvoke({
		get_all_ideas: () => ({ ideas: [] }),
		get_runtime_status: () => emptyStatus,
		list_models: () => emptyModels,
		get_ai_settings: () => emptyAiSettings,
		get_free_disk_space: () => 1_000_000_000_000,
		...extra
	});
}

describe("DashboardSection", () => {
	it("renders the error state with retry when the library fails to load", async () => {
		mockInvoke({
			get_all_ideas: () => {
				throw new Error("boom");
			}
		});
		render(<DashboardSection />);
		expect(await screen.findByText("Couldn't load your ideas")).toBeInTheDocument();
		expect(screen.getByText("Try again")).toBeInTheDocument();
	});

	it("shows the library grid for an imported-only library", async () => {
		// every idea has a creator: nothing was authored here, but the
		// library is not empty, so the grid (not onboarding) must show
		mockBase({
			get_all_ideas: () => ({
				ideas: [
					{
						id: "imp1",
						title: "Imported",
						result: "summary text",
						creator_name: "Ada",
						is_unread: true,
						feedback: []
					}
				]
			})
		});
		render(<DashboardSection />);
		await waitFor(() =>
			expect(screen.queryByText("Start your first Brainstory!")).not.toBeInTheDocument()
		);
		expect(await screen.findByText("Imported")).toBeInTheDocument();
	});

	it("shows onboarding only for an empty library", async () => {
		mockBase();
		render(<DashboardSection />);
		expect(await screen.findByText("Start your first Brainstory!")).toBeInTheDocument();
	});

	it("a successful import refreshes the library", async () => {
		const user = userEvent.setup();
		let importCount = 0;
		mockBase({
			import_share: () => {
				importCount++;
				return {
					cancelled: false,
					kind: "idea",
					id: "imp2",
					title: "New import",
					author: "Ada"
				};
			}
		});
		render(<DashboardSection />);
		await user.click(
			await screen.findByRole("button", { name: "Import shared idea or feedback" })
		);
		expect(await screen.findByText(/Imported "New import" from Ada/)).toBeInTheDocument();
		await waitFor(() => expect(importCount).toBe(1));
		// the refresh re-fetched the library after the import
		await waitFor(() =>
			expect(
				vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "get_all_ideas").length
			).toBeGreaterThanOrEqual(2)
		);
	});
});

describe("DashboardSection feedback drafts", () => {
	const party = {
		id: "party",
		title: "Idea for a Surprise Birthday Party",
		result: "# Idea for a Surprise Birthday Party\n\n## Plan\nbody",
		type: "original",
		created_at: "2026-09-01T10:00:00",
		feedback: [
			{
				id: "fb-draft",
				title: "",
				result: "",
				type: "feedback",
				created_at: "2026-09-03T10:00:00",
				transcript: [
					{ role: "assistant", content: "What do you think?" },
					{ role: "user", content: "invite the neighbours" }
				]
			},
			{
				id: "fb-done",
				title: "Thoughts",
				result: "## Thoughts",
				type: "feedback",
				created_at: "2026-09-02T10:00:00"
			}
		]
	};
	const ownDraft = {
		id: "own-draft",
		title: "",
		result: "",
		type: "original",
		created_at: "2026-09-02T12:00:00",
		transcript: [{ role: "user", content: "a half idea" }],
		feedback: []
	};

	function feedbackDraftLink() {
		return screen.findByRole("link", {
			name: /Feedback on: Idea for a Surprise Birthday Party/
		});
	}

	it("shows an unfinished feedback chat as a draft card that resumes it", async () => {
		mockBase({ get_all_ideas: () => ({ ideas: [party, ownDraft] }) });
		render(<DashboardSection />);
		const link = await feedbackDraftLink();
		expect(link).toHaveAttribute("href", "/chat?parentId=party&id=fb-draft");
		expect(link).toHaveTextContent("Feedback draft");
		expect(link).toHaveTextContent("invite the neighbours");
		// an ordinary draft still resumes as an original idea
		expect(screen.getByRole("link", { name: /a half idea/ })).toHaveAttribute(
			"href",
			"/chat?id=own-draft"
		);
		// placed among the drafts by date: the feedback draft is the newest
		const links = screen.getAllByRole("link").map((a) => a.getAttribute("href"));
		expect(links.indexOf("/chat?parentId=party&id=fb-draft")).toBeLessThan(
			links.indexOf("/chat?id=own-draft")
		);
		expect(links.indexOf("/chat?id=own-draft")).toBeLessThan(links.indexOf("/idea?id=party"));
	});

	it("does not count the draft as feedback on its idea's card", async () => {
		mockBase({ get_all_ideas: () => ({ ideas: [party] }) });
		render(<DashboardSection />);
		await feedbackDraftLink();
		expect(screen.getByRole("link", { name: "1 feedback item" })).toHaveAttribute(
			"href",
			"/idea?id=party&tab=feedback"
		);
	});

	it("deletes the feedback draft from its card and drops it from the grid", async () => {
		const user = userEvent.setup();
		mockBase({
			get_all_ideas: () => ({ ideas: [party] }),
			delete_idea: () => null
		});
		render(<DashboardSection />);
		await feedbackDraftLink();
		const remove = screen.getByRole("button", { name: "Delete draft" });
		await user.click(remove);
		await user.click(screen.getByRole("button", { name: "Confirm delete draft" }));
		await waitFor(() =>
			expect(screen.queryByRole("link", { name: /Feedback on:/ })).not.toBeInTheDocument()
		);
		const deleted = vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "delete_idea");
		expect(deleted).toEqual([["delete_idea", { ideaId: "fb-draft" }]]);
		// the idea itself stays
		expect(
			screen.getByRole("link", { name: /Idea for a Surprise Birthday Party/ })
		).toHaveAttribute("href", "/idea?id=party");
	});
});

describe("DashboardSection onboarding flag", () => {
	afterEach(() => {
		localStorage.clear();
	});

	it("sets the getting-started flag once an own idea exists", async () => {
		mockBase({
			get_all_ideas: () => ({
				ideas: [{ id: "own1", title: "Mine", result: "summary", feedback: [] }]
			})
		});
		render(<DashboardSection />);
		expect(await screen.findByText("Mine")).toBeInTheDocument();
		expect(hasDoneGettingStarted()).toBe(true);
	});

	it.each([
		["an empty library", []],
		[
			"an imported-only library",
			[{ id: "imp1", title: "Imported", result: "s", creator_name: "Ada", feedback: [] }]
		]
	])(
		"deleting every own idea (%s) does not unset it - onboarding stays done",
		async (_label, ideas) => {
			markGettingStartedDone();
			mockBase({ get_all_ideas: () => ({ ideas }) });
			render(<DashboardSection />);
			await waitFor(() =>
				expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "get_all_ideas")).toBe(
					true
				)
			);
			// let the post-load effects run
			await screen.findByRole("heading", { name: "Dashboard" });
			await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
			expect(hasDoneGettingStarted()).toBe(true);
		}
	);
});
