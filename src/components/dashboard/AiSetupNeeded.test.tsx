import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";

import { mockInvoke } from "@src/test/mock-tauri";
import { invoke } from "@tauri-apps/api/core";
import { $aiStatus, type AiStatus } from "@components/global/aiStatusStore";
import AiSetupNeeded from "./AiSetupNeeded";

const GiB = 1024 ** 3;
const models = {
	llm: [
		{ id: "big", label: "Big", sizeBytes: 4 * GiB, downloaded: false },
		{ id: "small", label: "Small", sizeBytes: 1.5 * GiB, downloaded: false }
	],
	stt: []
};

function setLlm(state: string): void {
	$aiStatus.set({ llm: { state }, stt: { state: "ready" } } as AiStatus);
}

function commands(): string[] {
	return vi.mocked(invoke).mock.calls.map(([cmd]) => cmd);
}

let runtimeLlmState = "missing";

beforeEach(() => {
	runtimeLlmState = "missing";
	mockInvoke({
		get_runtime_status: () => ({ llm: { state: runtimeLlmState }, stt: { state: "ready" } }),
		list_models: () => models
	});
});

describe("AiSetupNeeded", () => {
	it("shows the setup card when the LLM engine is missing, sized from the catalog", async () => {
		setLlm("missing");
		render(<AiSetupNeeded />);
		expect(await screen.findByText(/One quick step/)).toBeInTheDocument();
		expect(screen.getByText(/~1\.5 GB, once/)).toBeInTheDocument();
		// derived from the shared store: no per-mount settings refetch
		expect(commands()).not.toContain("get_ai_settings");
	});

	it.each(["ready", "loading", "external", "error", "unknown"])(
		"stays hidden when the LLM engine is %s (no catalog fetch)",
		async (state) => {
			runtimeLlmState = state;
			setLlm(state);
			render(<AiSetupNeeded />);
			await act(async () => {});
			expect(screen.queryByText(/One quick step/)).not.toBeInTheDocument();
			expect(commands()).not.toContain("list_models");
		}
	);

	it("stays hidden when missing only because another downloaded model is not selected", async () => {
		mockInvoke({
			get_runtime_status: () => ({ llm: { state: "missing" }, stt: { state: "ready" } }),
			list_models: () => ({
				llm: [{ ...models.llm[0], downloaded: true }, models.llm[1]],
				stt: []
			})
		});
		setLlm("missing");
		render(<AiSetupNeeded />);
		await act(async () => {});
		expect(screen.queryByText(/One quick step/)).not.toBeInTheDocument();
	});

	it("follows live status: the card disappears once a model becomes ready", async () => {
		setLlm("missing");
		render(<AiSetupNeeded />);
		expect(await screen.findByText(/One quick step/)).toBeInTheDocument();
		act(() => setLlm("ready"));
		expect(screen.queryByText(/One quick step/)).not.toBeInTheDocument();
	});
});
