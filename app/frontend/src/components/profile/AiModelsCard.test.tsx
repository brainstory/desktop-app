import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import { useState } from "react";
import userEvent from "@testing-library/user-event";

import { mockInvoke } from "@src/test/mock-tauri";
import { invoke } from "@tauri-apps/api/core";
import { listen, type EventCallback } from "@tauri-apps/api/event";
import AiModelsCard from "./AiModelsCard";

const aiSettings = {
	llmMode: "local",
	llmModel: "gemma-4-E2B-qat",
	sttModel: "whisper-tiny-en",
	sttEngine: "whisper",
	sttLanguage: "en-US",
	hfTokenSet: false,
	hfTokenHint: null as string | null,
	hfEndpoint: "",
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

/** Capture the backend event handlers the card subscribes to. */
function captureListeners(): Record<string, EventCallback<unknown>> {
	const handlers: Record<string, EventCallback<unknown>> = {};
	vi.mocked(listen).mockImplementation(async (event, handler) => {
		handlers[event] = handler as EventCallback<unknown>;
		return () => {};
	});
	return handlers;
}

function emit(handlers: Record<string, EventCallback<unknown>>, event: string, payload: unknown) {
	act(() => {
		handlers[event]!({ event, id: 0, payload });
	});
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
	afterEach(() => {
		vi.mocked(listen).mockReset();
		vi.mocked(listen).mockImplementation(async () => () => {});
	});

	it("refuses to switch to the external LLM without a URL and stays off", async () => {
		const user = userEvent.setup();
		mockCard();
		renderCard();
		const sw = await screen.findByRole("switch", { name: undefined });
		// the switch sits next to its label text
		expect(screen.getByText("Use external LLM endpoint").parentElement).toContainElement(sw);
		expect(sw).toHaveAttribute("aria-checked", "false");
		await user.click(sw);
		expect(
			await screen.findByText("Set an external endpoint URL first, then enable this")
		).toBeInTheDocument();
		// controlled: the switch did NOT flip
		expect(sw).toHaveAttribute("aria-checked", "false");
		expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "save_ai_settings")).toBe(
			false
		);
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
			const call = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "save_ai_settings");
			expect(call, "save_ai_settings was called").toBeTruthy();
			expect(call![1]).toEqual({ ai: { hfToken: "hf_secret" } });
		});
	});

	it("saving the mirror endpoint sends only that field", async () => {
		const user = userEvent.setup();
		mockCard();
		renderCard();
		const input = await screen.findByLabelText(/download endpoint/i);
		await user.type(input, "https://hf-mirror.com");
		const row = input.parentElement!;
		const save = within(row).getByRole("button", { name: "Save" });
		await user.click(save);
		await waitFor(() => {
			const call = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "save_ai_settings");
			expect(call, "save_ai_settings was called").toBeTruthy();
			expect(call![1]).toEqual({ ai: { hfEndpoint: "https://hf-mirror.com" } });
		});
	});

	it("a toggle sends only its own key and reverts with a message when the save fails", async () => {
		const user = userEvent.setup();
		mockCard({
			get_ai_settings: () => ({ ...aiSettings, extLlmBaseUrl: "http://localhost:11434" }),
			save_ai_settings: () => {
				throw new Error("disk full");
			}
		});
		renderCard();
		const sw = await screen.findByRole("switch", { name: "Use external LLM endpoint" });
		// a half-typed endpoint field must not ride along with the toggle
		await user.type(screen.getByLabelText("STT base URL"), "localh");
		await user.click(sw);
		await waitFor(() => {
			const call = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "save_ai_settings");
			expect(call, "save_ai_settings was called").toBeTruthy();
			expect(call![1]).toEqual({ ai: { llmMode: "external" } });
		});
		expect(await screen.findByText("disk full")).toBeInTheDocument();
		// the optimistic flip is undone, the typed field is kept
		expect(sw).toHaveAttribute("aria-checked", "false");
		expect(screen.getByLabelText("STT base URL")).toHaveValue("localh");
	});

	it("a stale stored value does not block unrelated saves", async () => {
		const user = userEvent.setup();
		mockCard({
			// stored before URL validation existed; the backend now rejects it
			get_ai_settings: () => ({ ...aiSettings, extLlmBaseUrl: "localhost:1234" }),
			save_ai_settings: (args) => {
				const ai = (args as { ai: Record<string, unknown> }).ai;
				if ("extLlmBaseUrl" in ai) {
					throw new Error("invalid extLlmBaseUrl 'localhost:1234'");
				}
				return undefined;
			}
		});
		renderCard();
		await user.click(await screen.findByRole("button", { name: "Auto" }));
		await waitFor(() => {
			const call = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "save_ai_settings");
			expect(call![1]).toEqual({ ai: { sttEngine: "auto" } });
		});
		expect(screen.queryByText(/invalid extLlmBaseUrl/)).not.toBeInTheDocument();
	});

	it("Save Endpoints sends only the edited endpoint fields", async () => {
		const user = userEvent.setup();
		mockCard({
			get_ai_settings: () => ({
				...aiSettings,
				// stale values the backend would now reject
				llmModel: "removed-model",
				extLlmBaseUrl: "localhost:1234"
			})
		});
		renderCard();
		await user.type(await screen.findByLabelText("STT model"), "whisper-1");
		await user.click(screen.getByRole("button", { name: "Save Endpoints" }));
		await waitFor(() => {
			const call = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "save_ai_settings");
			expect(call, "save_ai_settings was called").toBeTruthy();
			expect(call![1]).toEqual({ ai: { extSttModel: "whisper-1" } });
		});
		expect(await screen.findByRole("button", { name: "All changes saved" })).toBeDisabled();
	});

	it("enabling the external LLM needs a saved URL, not just a typed one", async () => {
		const user = userEvent.setup();
		mockCard();
		renderCard();
		const sw = await screen.findByRole("switch", { name: "Use external LLM endpoint" });
		await user.type(screen.getByLabelText("LLM base URL"), "http://localhost:11434");
		await user.click(sw);
		expect(
			await screen.findByText("Save the external endpoint URL first, then enable this")
		).toBeInTheDocument();
		expect(sw).toHaveAttribute("aria-checked", "false");
		expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "save_ai_settings")).toBe(
			false
		);
	});

	it("offers the speech language for a multilingual whisper model", async () => {
		const user = userEvent.setup();
		const turbo = {
			...models.stt[0]!,
			id: "whisper-large-v3-turbo",
			label: "Whisper large v3 turbo"
		};
		mockCard({
			get_ai_settings: () => ({ ...aiSettings, sttModel: turbo.id }),
			list_models: () => ({ ...models, stt: [turbo] })
		});
		renderCard();
		const select = await screen.findByLabelText("Speech language");
		expect(
			screen.getByText(/Whisper large v3 turbo transcribes in this language/)
		).toBeInTheDocument();
		expect(screen.queryByText(/Applies to the Apple Speech engine/)).not.toBeInTheDocument();
		await user.selectOptions(select, "de-DE");
		await waitFor(() => {
			const call = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "save_ai_settings");
			expect(call![1]).toEqual({ ai: { sttLanguage: "de-DE" } });
		});
	});

	it("hides the speech language for an English-only whisper model", async () => {
		mockCard();
		renderCard();
		await screen.findByText("Whisper tiny (English)");
		expect(screen.queryByLabelText("Speech language")).not.toBeInTheDocument();
	});

	it("deleting a model needs a second click within a visible countdown", async () => {
		const user = userEvent.setup();
		const inactive = {
			...models.llm[0]!,
			id: "minicpm5-2b",
			label: "MiniCPM5 2B",
			active: false
		};
		mockCard({
			list_models: () => ({ ...models, llm: [models.llm[0]!, inactive] }),
			delete_model: () => undefined
		});
		renderCard();
		await screen.findByText("MiniCPM5 2B");
		const [activeDelete, inactiveDelete] = screen.getAllByRole("button", { name: "Delete" });

		await user.click(inactiveDelete!);
		expect(inactiveDelete).toHaveTextContent("Really delete? (5s)");
		expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "delete_model")).toBe(false);
		await user.click(inactiveDelete!);
		await waitFor(() =>
			expect(vi.mocked(invoke)).toHaveBeenCalledWith("delete_model", {
				modelId: "minicpm5-2b"
			})
		);

		// the active model keeps its stronger warning
		await user.click(activeDelete!);
		expect(activeDelete).toHaveTextContent("Really delete the ACTIVE model? (5s)");
	});

	it("a progress event updates the download bar; an unknown size is indeterminate", async () => {
		const handlers = captureListeners();
		mockCard();
		renderCard();
		await screen.findByText("Whisper tiny (English)");
		await waitFor(() => expect(handlers["model-download"]).toBeDefined());

		emit(handlers, "model-download", {
			modelId: "whisper-tiny-en",
			kind: "progress",
			pct: 42.7
		});
		const bar = screen.getByRole("progressbar", {
			name: "Whisper tiny (English) download progress"
		});
		expect(bar).toHaveAttribute("aria-valuenow", "42");
		expect(screen.getByText("42%")).toBeInTheDocument();

		// -1 = the backend could not determine the total size
		emit(handlers, "model-download", {
			modelId: "whisper-tiny-en",
			kind: "progress",
			pct: -1
		});
		const indeterminate = screen.getByRole("progressbar", {
			name: "Whisper tiny (English) download progress"
		});
		expect(indeterminate).not.toHaveAttribute("aria-valuenow");
		expect(screen.queryByText("-1%")).not.toBeInTheDocument();
		expect(screen.getByText("…")).toBeInTheDocument();
	});
});
