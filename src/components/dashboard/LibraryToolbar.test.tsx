import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, renderHook, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import LibraryToolbar, { useLibraryView } from "./LibraryToolbar";
import { STORAGE_KEYS } from "@src/tauri/commands";
import type { LibraryFilter } from "@helpers/ideas";
import type { IdeaListItem } from "@src/types";

const library: IdeaListItem[] = [
	{ id: "a", summaryPreview: "", title: "Ampersand", createdAt: "2026-01-01" },
	{ id: "b", summaryPreview: "", title: "Buffer", createdAt: "2026-02-01", isDraft: true },
	{
		id: "c",
		summaryPreview: "",
		title: "Cached",
		createdAt: "2026-03-01",
		creatorName: "Ada"
	}
];

describe("LibraryToolbar", () => {
	beforeEach(() => {
		localStorage.clear();
	});
	afterEach(() => {
		localStorage.clear();
	});

	it("shows the idea count and toggles chips via the props", async () => {
		const props = {
			count: library.length,
			total: library.length,
			query: "",
			onQueryChange: vi.fn(),
			filters: [] as LibraryFilter[],
			onFiltersChange: vi.fn(),
			sort: "newest" as const,
			onSortChange: vi.fn()
		};
		render(<LibraryToolbar {...props} />);
		expect(screen.getByText("3 of 3 ideas")).toBeInTheDocument();

		await userEvent.click(screen.getByRole("button", { name: "Drafts" }));
		expect(props.onFiltersChange).toHaveBeenCalledWith(["drafts"]);
	});

	it("offers the sort orders", async () => {
		const props = {
			count: library.length,
			total: library.length,
			query: "",
			onQueryChange: vi.fn(),
			filters: [] as LibraryFilter[],
			onFiltersChange: vi.fn(),
			sort: "newest" as const,
			onSortChange: vi.fn()
		};
		render(<LibraryToolbar {...props} />);
		await userEvent.selectOptions(
			screen.getByRole("combobox", { name: "Sort ideas" }),
			"title"
		);
		expect(props.onSortChange).toHaveBeenCalledWith("title");
	});

	it("focuses the search box on Cmd+F", async () => {
		render(
			<LibraryToolbar
				count={1}
				total={1}
				query=""
				onQueryChange={() => {}}
				filters={[]}
				onFiltersChange={() => {}}
				sort="newest"
				onSortChange={() => {}}
			/>
		);
		const search = screen.getByRole("searchbox", { name: "Search ideas" });
		expect(search).not.toHaveFocus();
		await userEvent.keyboard("{Meta>}{f}{/Meta}");
		expect(search).toHaveFocus();
	});
});

describe("useLibraryView", () => {
	beforeEach(() => {
		localStorage.clear();
	});
	afterEach(() => {
		localStorage.clear();
	});

	it("filters the library and reports the empty-filter state", () => {
		const { result } = renderHook(() => useLibraryView(library));
		expect(result.current.visible).toHaveLength(3);
		expect(result.current.isFiltering).toBe(false);

		// the toolbar's handlers are the hook's own setters
		act(() => result.current.setQuery("buff"));
		expect(result.current.visible.map((idea) => idea.id)).toEqual(["b"]);
		expect(result.current.isFiltering).toBe(true);

		act(() => result.current.setQuery("nothing-matches"));
		expect(result.current.visible).toHaveLength(0);

		act(() => result.current.setQuery(""));
		expect(result.current.visible).toHaveLength(3);
	});

	it("persists the sort and chips and restores them on the next mount", () => {
		const { result } = renderHook(() => useLibraryView(library));
		act(() => {
			result.current.setSort("oldest");
			result.current.setFilters(["unread"]);
		});
		expect(localStorage.getItem(STORAGE_KEYS.librarySort)).toBe("oldest");
		expect(localStorage.getItem(STORAGE_KEYS.librarySortFilters)).toBe("unread");

		const { result: next } = renderHook(() => useLibraryView(library));
		expect(next.current.sort).toBe("oldest");
		expect(next.current.filters).toEqual(["unread"]);
	});
});
