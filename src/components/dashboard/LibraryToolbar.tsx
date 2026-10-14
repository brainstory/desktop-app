import { useEffect, useRef, useState } from "react";
import {
	LIBRARY_FILTERS,
	LIBRARY_SORTS,
	filterAndSortIdeas,
	type LibraryFilter,
	type LibrarySort
} from "@helpers/ideas";
import { STORAGE_KEYS } from "@src/tauri/commands";

/** localStorage with the app's usual resilience: a broken store must
 * never break the page. */
const readStored = (key: string): string | null => {
	try {
		return localStorage.getItem(key);
	} catch {
		return null;
	}
};
const writeStored = (key: string, value: string): void => {
	try {
		localStorage.setItem(key, value);
	} catch {
		// preferences just won't persist
	}
};

const readSort = (): LibrarySort =>
	LIBRARY_SORTS.find((option) => option.value === readStored(STORAGE_KEYS.librarySort))?.value ??
	"newest";

const readFilters = (): LibraryFilter[] => {
	const raw = readStored(STORAGE_KEYS.librarySortFilters) ?? "";
	return raw
		.split(",")
		.map((value) => value.trim())
		.filter((value): value is LibraryFilter =>
			LIBRARY_FILTERS.some((option) => option.value === value)
		);
};

interface LibraryToolbarProps {
	count: number;
	total: number;
	query: string;
	onQueryChange: (query: string) => void;
	filters: LibraryFilter[];
	onFiltersChange: (filters: LibraryFilter[]) => void;
	sort: LibrarySort;
	onSortChange: (sort: LibrarySort) => void;
}

/**
 * Search, filter chips and sort for the library. Pure presentational:
 * the filtering itself lives in `filterAndSortIdeas`, and the sort and
 * chips persist in local storage.
 */
export default function LibraryToolbar({
	count,
	total,
	query,
	onQueryChange,
	filters,
	onFiltersChange,
	sort,
	onSortChange
}: LibraryToolbarProps) {
	const searchRef = useRef<HTMLInputElement | null>(null);

	// Cmd/Ctrl+F jumps to the search box (the browser's own find is
	// useless for cards that render previews only)
	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.key.toLowerCase() === "f" && (event.metaKey || event.ctrlKey)) {
				event.preventDefault();
				searchRef.current?.focus();
				searchRef.current?.select();
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, []);

	const toggleFilter = (filter: LibraryFilter): void => {
		onFiltersChange(
			filters.includes(filter)
				? filters.filter((value) => value !== filter)
				: [...filters, filter]
		);
	};

	return (
		<div className="flex flex-col gap-2 mb-6">
			<div className="flex flex-wrap items-center gap-2">
				<input
					ref={searchRef}
					type="search"
					value={query}
					onChange={(event) => onQueryChange(event.target.value)}
					placeholder="Search ideas"
					aria-label="Search ideas"
					className="grow max-w-xs px-3 py-2 text-sm border border-stone-300 rounded-md focus:outline-none focus:border-accent-500"
				/>
				{LIBRARY_FILTERS.map(({ value, label }) => (
					<button
						key={value}
						type="button"
						aria-pressed={filters.includes(value)}
						onClick={() => toggleFilter(value)}
						className={`px-3 py-1.5 text-xs font-medium rounded-full border transition-colors ${
							filters.includes(value)
								? "bg-accent-600 text-white border-accent-600"
								: "bg-white text-stone-600 border-stone-300 hover:border-stone-400"
						}`}
					>
						{label}
					</button>
				))}
				<select
					value={sort}
					onChange={(event) => onSortChange(event.target.value as LibrarySort)}
					aria-label="Sort ideas"
					className="ml-auto px-2 py-1.5 text-sm border border-stone-300 rounded-md bg-white"
				>
					{LIBRARY_SORTS.map(({ value, label }) => (
						<option key={value} value={value}>
							{label}
						</option>
					))}
				</select>
			</div>
			<p className="text-xs text-stone-500">
				{count} of {total} {total === 1 ? "idea" : "ideas"}
			</p>
		</div>
	);
}

/** The toolbar plus the filtering state it drives: what the dashboard
 * renders above the grid. */
export function useLibraryView(ideas: ReturnType<typeof filterAndSortIdeas>) {
	const [query, setQuery] = useState("");
	const [filters, setFilters] = useState<LibraryFilter[]>(readFilters);
	const [sort, setSort] = useState<LibrarySort>(readSort);

	useEffect(() => {
		writeStored(STORAGE_KEYS.librarySort, sort);
	}, [sort]);
	useEffect(() => {
		writeStored(STORAGE_KEYS.librarySortFilters, filters.join(","));
	}, [filters]);

	const visible = filterAndSortIdeas(ideas, query, filters, sort);
	const isFiltering = query.trim() !== "" || filters.length > 0;
	const clear = (): void => {
		setQuery("");
		setFilters([]);
	};

	return {
		visible,
		isFiltering,
		clear,
		query,
		filters,
		sort,
		setQuery,
		setFilters,
		setSort,
		toolbar: (
			<LibraryToolbar
				count={visible.length}
				total={ideas.length}
				query={query}
				onQueryChange={setQuery}
				filters={filters}
				onFiltersChange={setFilters}
				sort={sort}
				onSortChange={setSort}
			/>
		)
	};
}
