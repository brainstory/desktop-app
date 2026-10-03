import { describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { mockInvoke } from "@src/test/mock-tauri";

import { useIdeaReactions } from "./useIdeaReactions";

describe("useIdeaReactions", () => {
	it("applies only the latest answer when two toggles race", async () => {
		const answers: ((isOn: boolean) => void)[] = [];
		mockInvoke({
			get_reactions: () => ({ sections: [], comments: [] }),
			toggle_section_reaction: () =>
				new Promise<boolean>((resolve) => {
					answers.push(resolve);
				})
		});
		const { result } = renderHook(() => useIdeaReactions("i1", () => {}));
		await waitFor(() => expect(result.current.sectionReactions).toEqual([]));

		act(() => result.current.toggleSectionReaction(2, "👍")); // on
		expect(result.current.sectionReactions).toHaveLength(1);
		act(() => result.current.toggleSectionReaction(2, "👍")); // off again
		expect(result.current.sectionReactions).toHaveLength(0);

		// the second (latest) answer arrives first, the stale one after it
		await act(async () => answers[1]!(false));
		await act(async () => answers[0]!(true));
		expect(result.current.sectionReactions).toEqual([]);
	});

	it("keeps the page usable when loading reactions fails", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		mockInvoke({
			get_reactions: () => {
				throw "no such table";
			}
		});
		const { result } = renderHook(() => useIdeaReactions("i1", () => {}));
		await waitFor(() => expect(console.error).toHaveBeenCalled());
		expect(result.current.sectionReactions).toEqual([]);
		vi.restoreAllMocks();
	});
});
