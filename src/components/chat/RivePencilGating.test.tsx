import { describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";

import { mockViewport } from "@src/test/match-media";
import { AssistantResponseText } from "./AssistantResponseText";
import GetStartedIntro from "./GetStartedIntro";

// stand-in for the Rive canvas: mounting it is what pulls in the WASM
vi.mock("@components/global/RivePencil", () => ({
	default: () => <div data-testid="rive-pencil" />
}));

describe("RivePencil gating", () => {
	it("AssistantResponseText mounts the pencil only on wide windows", () => {
		const viewport = mockViewport(800);
		render(<AssistantResponseText content="What is on your mind?" />);
		expect(screen.queryByTestId("rive-pencil")).not.toBeInTheDocument();

		act(() => viewport.setWidth(1200));
		expect(screen.getByTestId("rive-pencil")).toBeInTheDocument();

		act(() => viewport.setWidth(900));
		expect(screen.queryByTestId("rive-pencil")).not.toBeInTheDocument();
	});

	it("GetStartedIntro mounts the pencil only on wide windows", () => {
		const viewport = mockViewport(800);
		render(<GetStartedIntro />);
		expect(screen.queryByTestId("rive-pencil")).not.toBeInTheDocument();

		act(() => viewport.setWidth(1024));
		expect(screen.getByTestId("rive-pencil")).toBeInTheDocument();
	});
});
