import { onTestFinished, vi } from "vitest";

type Listener = (event: MediaQueryListEvent) => void;

interface FakeList {
	query: string;
	matches: boolean;
	listeners: Set<Listener>;
}

/**
 * Controllable `window.matchMedia` for one test: queries are evaluated
 * against a fake viewport width (only `min-width` / `max-width` in px),
 * and `setWidth` fires `change` on every query whose result flips.
 * Call it inside the test; the spy is restored when the test finishes.
 *
 *   const viewport = mockViewport(1024);
 *   act(() => viewport.setWidth(500));
 */
export function mockViewport(initialWidth: number): { setWidth: (width: number) => void } {
	let width = initialWidth;
	const lists = new Set<FakeList>();

	const evaluate = (query: string): boolean => {
		const min = /min-width:\s*(\d+)px/.exec(query);
		const max = /max-width:\s*(\d+)px/.exec(query);
		if (min && width < Number(min[1])) return false;
		if (max && width > Number(max[1])) return false;
		return Boolean(min || max);
	};

	const spy = vi.spyOn(window, "matchMedia").mockImplementation((query: string) => {
		const list: FakeList = { query, matches: evaluate(query), listeners: new Set() };
		lists.add(list);
		return {
			get matches() {
				return evaluate(query);
			},
			media: query,
			onchange: null,
			addListener: (fn: Listener) => list.listeners.add(fn),
			removeListener: (fn: Listener) => list.listeners.delete(fn),
			addEventListener: (_type: string, fn: Listener) => list.listeners.add(fn),
			removeEventListener: (_type: string, fn: Listener) => list.listeners.delete(fn),
			dispatchEvent: () => false
		} as unknown as MediaQueryList;
	});

	onTestFinished(() => spy.mockRestore());

	return {
		setWidth(next: number) {
			width = next;
			for (const list of lists) {
				const matches = evaluate(list.query);
				if (matches === list.matches) continue;
				list.matches = matches;
				for (const fn of list.listeners) {
					fn({ matches, media: list.query } as MediaQueryListEvent);
				}
			}
		}
	};
}
