import { describe, expect, it } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { renderToString } from "react-dom/server";

import { mockViewport } from "@src/test/match-media";
import { useMediaQuery } from "./useMediaQuery";

function Probe({ serverValue }: { serverValue?: boolean }) {
	const wide = useMediaQuery("(min-width: 1024px)", serverValue);
	return <p>{wide ? "wide" : "narrow"}</p>;
}

describe("useMediaQuery", () => {
	it("reads the current match and follows changes", () => {
		const viewport = mockViewport(1200);
		render(<Probe />);
		expect(screen.getByText("wide")).toBeInTheDocument();
		act(() => viewport.setWidth(800));
		expect(screen.getByText("narrow")).toBeInTheDocument();
		act(() => viewport.setWidth(1100));
		expect(screen.getByText("wide")).toBeInTheDocument();
	});

	it("renders the server value during SSR", () => {
		expect(renderToString(<Probe serverValue={true} />)).toContain("wide");
		expect(renderToString(<Probe serverValue={false} />)).toContain("narrow");
	});
});
