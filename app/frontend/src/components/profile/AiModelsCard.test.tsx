import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import { useState } from "react";
import userEvent from "@testing-library/user-event";

import { mockInvoke } from "@src/test/mock-tauri";
import { invoke } from "@tauri-apps/api/core";
import AiModelsCard from "./AiModelsCard";

const aiSettings = {
	llmMode: "local",
	llmModel: "gemma-4-E2B-qat",
	sttModel: "whisper-tiny-en",
	sttEngine: "whisper",
	sttLanguage: "en-US",
	hfTokenSet: false,
	hfTokenHint: null as string | null,
	extLlmBaseUrl: "",
	extLlmApiKeySet: false,
	extLlmApiKeyHint: null as string | null,
	extLlmModel: "",
	extSttBaseUrl: "",
	extSttApiKeySet: false,
	extSttApiKeyHint: null as string | null,
	extSttModel: ""
};

const models = {
	llm: [
		{
			id: "gemma-4-E2B-qat",
			label: "Gemma 4 E2B (light)",
			description: "desc",
			kind: "llm" as const,
			sizeBytes: 1,
			downloaded: true,
			active: true,
			downloading: false,
			progress: null as number | null,
			filename: "m.gguf"
		}
	],
	stt: [
		{
			id: "whisper-tiny-en",
			label: "Whisper tiny (English)",
			description: "desc",
			kind: "stt" as const,
			sizeBytes: 1,
			downloaded: true,
			active: true,
			downloading: false,
			progress: null as number | null,
			filename: "t.bin"
		}
	]
};

/** Render the card with a host that surfaces snackbar messages in the DOM. */
function renderCard() {
	function Host() {
		const [message, setMessage] = useState<string | null>(null);
		return (
			<>
				{message && <p>{message}</p>}
				<AiModelsCard openSnackbar={(_ok, msg) => setMessage(msg)} />
			</>
		);
	}
	return render(<Host />);
}

function mockCard(extra: Record<string, (args: unknown) => unknown> = {}) {
	mockInvoke({
		get_ai_settings: () => aiSettings,
		list_models: () => models,
		get_runtime_status: () => ({
			llm: { state: "ready", model_id: "gemma-4-E2B-qat" },
			stt: { state: "ready", model_id: "whisper-tiny-en" }
		}),
		get_apple_stt_status: () => ({
			available: false,
			authorized: false,
			supportedLocales: [],
			installedLocales: []
		}),
		get_free_disk_space: () => 1_000_000_000_000,
		save_ai_settings: () => undefined,
		...extra
	});
}

describe("AiModelsCard", () => {
	it("refuses to switch to the external LLM without a URL and stays off", async () => {
		const user = userEvent.setup();
		mockCard();
		renderCard();
		const sw = await screen.findByRole("switch", { name: undefined });
		// the switch sits next to its label text
		expect(
			screen.getByText("Use external LLM endpoint").parentElement
		).toContainElement(sw);
		expect(sw).toHaveAttribute("aria-checked", "false");
		await user.click(sw);
		expect(
			await screen.findByText("Set an external endpoint URL first, then enable this")
		).toBeInTheDocument();
		// controlled: the switch did NOT flip
		expect(sw).toHaveAttribute("aria-checked", "false");
		expect(
			vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "save_ai_settings")
		).toBe(false);
	});

	it("saving a secret sends only that field", async () => {
		const user = userEvent.setup();
		mockCard();
		renderCard();
		const hfInput = await screen.findByLabelText(/HuggingFace access token/);
		await user.type(hfInput, "hf_secret");
		// the Save button scoped to the HF secret's own field row
		const fieldRow = hfInput.parentElement!;
		const saveButton = within(fieldRow).getByRole("button", { name: "Save Token" });
		await user.click(saveButton);
		await waitFor(() => {
			const call = vi
				.mocked(invoke)
				.mock.calls.find(([cmd]) => cmd === "save_ai_settings");
			expect(call, "save_ai_settings was called").toBeTruthy();
			expect(call![1]).toEqual({ ai: { hfToken: "hf_secret" } });
		});
	});
});
