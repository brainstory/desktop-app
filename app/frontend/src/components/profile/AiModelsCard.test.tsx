import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import { useState } from "react";
import userEvent from "@testing-library/user-event";

import { mockInvoke } from "@src/test/mock-tauri";
import { invoke } from "@tauri-apps/api/core";
import { listen, type EventCallback } from "@tauri-apps/api/event";
import { $aiStatus } from "@components/global/aiStatusStore";
import AiModelsCard from "./AiModelsCard";
import { useAiModels } from "./useAiModels";

const aiSettings = {
	llmMode: "local",
	llmModel: "gemma-4-E2B-qat",
	sttModel: "whisper-tiny-en",
	sttMode: "local",
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
		handlers[event] = handler;
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

type SecretControl = {
	name: string;
	secretKey: "hfToken" | "extLlmApiKey" | "extSttApiKey";
	hint: string;
	saveLabel: string;
	openInput: (user: ReturnType<typeof userEvent.setup>) => Promise<HTMLElement>;
};

/** The three secret controls on the card and how to reach each input. */
const SECRET_CONTROLS: SecretControl[] = [
	{
		name: "HuggingFace token",
		secretKey: "hfToken",
		hint: "••••hf42",
		saveLabel: "Save Token",
		openInput: async () => screen.findByLabelText(/HuggingFace access token/)
	},
	{
		name: "LLM API key",
		secretKey: "extLlmApiKey",
		hint: "••••llm99",
		saveLabel: "Save",
		openInput: async (user) => {
			const llm = await screen.findByRole("region", { name: /Language model/ });
			await user.click(within(llm).getByText("External LLM endpoint"));
			return within(llm).getByLabelText("LLM API key (if needed)");
		}
	},
	{
		name: "STT API key",
		secretKey: "extSttApiKey",
		hint: "••••stt7",
		saveLabel: "Save",
		openInput: async (user) => {
			const stt = await screen.findByRole("region", { name: /Speech-to-text/ });
			await user.click(within(stt).getByText("External STT endpoint"));
			return within(stt).getByLabelText("STT API key (if needed)");
		}
	}
];

/**
 * Backend stand-in for the secret controls: the settings response flips
 * its presence/hint pair the moment save_ai_settings accepts the secret
 * ("" clears it), so a refresh reflects the confirmed stored state.
 * `failSave` rejects saves; `failRefreshAfter` fails every settings read
 * after the Nth one (1 = only the mount read succeeds).
 */
function mockSecretBackend(
	secretKey: SecretControl["secretKey"],
	hint: string,
	opts: {
		initialStored?: boolean;
		failSave?: () => string | undefined;
		failRefreshAfter?: number;
	} = {}
) {
	let stored = opts.initialStored ?? false;
	let refreshes = 0;
	mockCard({
		get_ai_settings: () => {
			refreshes++;
			if (opts.failRefreshAfter !== undefined && refreshes > opts.failRefreshAfter) {
				throw new Error("settings db busy");
			}
			return {
				...aiSettings,
				[`${secretKey}Set`]: stored,
				[`${secretKey}Hint`]: stored ? hint : null
			};
		},
		save_ai_settings: (args) => {
			const failure = opts.failSave?.();
			if (failure) throw new Error(failure);
			const ai = (args as { ai: Record<string, unknown> }).ai;
			// "" is the backend's clear signal; any other value stores it
			stored = Object.values(ai).every((value) => value !== "");
		}
	});
}

describe("AiModelsCard", () => {
	afterEach(() => {
		$aiStatus.set({ llm: { state: "missing" }, stt: { state: "missing" } });
		vi.mocked(listen).mockReset();
		vi.mocked(listen).mockImplementation(async () => () => {});
	});

	it("refuses to switch to the external LLM without a URL and stays off", async () => {
		const user = userEvent.setup();
		mockCard();
		renderCard();
		const sw = await screen.findByRole("switch", { name: "Use external LLM endpoint" });
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

	it("saving an endpoint sends only its own edited fields", async () => {
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
		const stt = await screen.findByRole("region", { name: /Speech-to-text/ });
		await user.type(within(stt).getByLabelText("STT model"), "whisper-1");
		await user.click(within(stt).getByRole("button", { name: "Save STT endpoint" }));
		await waitFor(() => {
			const call = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "save_ai_settings");
			expect(call, "save_ai_settings was called").toBeTruthy();
			expect(call![1]).toEqual({ ai: { extSttModel: "whisper-1" } });
		});
		expect(await within(stt).findByRole("button", { name: "Endpoint saved" })).toBeDisabled();
	});

	it("each endpoint lives in its own section, downloads in theirs", async () => {
		mockCard();
		renderCard();
		const llm = await screen.findByRole("region", { name: /Language model/ });
		const stt = screen.getByRole("region", { name: /Speech-to-text/ });
		const downloads = screen.getByRole("region", { name: "Model downloads" });
		for (const label of ["LLM base URL", "LLM model", "LLM API key (if needed)"]) {
			expect(within(llm).getByLabelText(label)).toBeInTheDocument();
			expect(within(stt).queryByLabelText(label)).not.toBeInTheDocument();
		}
		for (const label of ["STT base URL", "STT model", "STT API key (if needed)"]) {
			expect(within(stt).getByLabelText(label)).toBeInTheDocument();
			expect(within(llm).queryByLabelText(label)).not.toBeInTheDocument();
		}
		expect(within(downloads).getByLabelText(/HuggingFace access token/)).toBeInTheDocument();
		expect(within(llm).queryByLabelText(/HuggingFace access token/)).not.toBeInTheDocument();
	});

	it("saving the LLM endpoint leaves a half-typed STT field alone", async () => {
		const user = userEvent.setup();
		mockCard();
		renderCard();
		const llm = await screen.findByRole("region", { name: /Language model/ });
		const stt = screen.getByRole("region", { name: /Speech-to-text/ });
		await user.type(within(stt).getByLabelText("STT base URL"), "localh");
		await user.type(within(llm).getByLabelText("LLM base URL"), "http://localhost:11434");
		await user.click(within(llm).getByRole("button", { name: "Save LLM endpoint" }));
		await waitFor(() => {
			const call = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "save_ai_settings");
			expect(call![1]).toEqual({ ai: { extLlmBaseUrl: "http://localhost:11434" } });
		});
		// the STT edit is still pending in its own section
		expect(within(stt).getByRole("button", { name: "Save STT endpoint" })).toBeEnabled();
	});

	it("says when transcription goes to the external STT endpoint", async () => {
		mockCard({
			get_ai_settings: () => ({
				...aiSettings,
				sttMode: "external",
				extSttBaseUrl: "http://localhost:8080"
			})
		});
		renderCard();
		const stt = await screen.findByRole("region", { name: /Speech-to-text/ });
		expect(
			within(stt).getByText(/Transcription uses the external STT endpoint below/)
		).toBeInTheDocument();
	});

	it("a saved STT URL alone keeps transcription on this computer", async () => {
		mockCard({
			get_ai_settings: () => ({ ...aiSettings, extSttBaseUrl: "http://localhost:8080" })
		});
		renderCard();
		const stt = await screen.findByRole("region", { name: /Speech-to-text/ });
		expect(
			within(stt).getByRole("switch", { name: "Use external STT endpoint" })
		).toHaveAttribute("aria-checked", "false");
		expect(
			within(stt).queryByText(/Transcription uses the external STT endpoint/)
		).not.toBeInTheDocument();
	});

	it("refuses to switch STT to external without a saved URL and opens its settings", async () => {
		const user = userEvent.setup();
		mockCard();
		renderCard();
		const stt = await screen.findByRole("region", { name: /Speech-to-text/ });
		const details = within(stt).getByText("External STT endpoint").closest("details")!;
		const sw = within(stt).getByRole("switch", { name: "Use external STT endpoint" });
		await user.click(sw);
		expect(
			await screen.findByText("Set an external STT endpoint URL first, then enable this")
		).toBeInTheDocument();
		expect(sw).toHaveAttribute("aria-checked", "false");
		expect(details).toHaveAttribute("open");
		expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === "save_ai_settings")).toBe(
			false
		);
	});

	it("the STT switch sends only sttMode once a URL is saved", async () => {
		const user = userEvent.setup();
		mockCard({
			get_ai_settings: () => ({ ...aiSettings, extSttBaseUrl: "http://localhost:8080" })
		});
		renderCard();
		const stt = await screen.findByRole("region", { name: /Speech-to-text/ });
		const sw = within(stt).getByRole("switch", { name: "Use external STT endpoint" });
		await user.click(sw);
		await waitFor(() => {
			const call = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === "save_ai_settings");
			expect(call![1]).toEqual({ ai: { sttMode: "external" } });
		});
		expect(sw).toHaveAttribute("aria-checked", "true");
	});

	it("opens the LLM endpoint settings when enabling external mode without a URL", async () => {
		const user = userEvent.setup();
		mockCard();
		renderCard();
		const llm = await screen.findByRole("region", { name: /Language model/ });
		const details = within(llm).getByText("External LLM endpoint").closest("details")!;
		expect(details).not.toHaveAttribute("open");
		await user.click(within(llm).getByRole("switch", { name: "Use external LLM endpoint" }));
		expect(details).toHaveAttribute("open");
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

	it("says the language also reaches a multilingual whisper fallback under Apple Speech", async () => {
		const turbo = {
			...models.stt[0]!,
			id: "whisper-large-v3-turbo",
			label: "Whisper large v3 turbo",
			// list_models marks no whisper model active while Apple Speech runs
			active: false
		};
		mockCard({
			get_ai_settings: () => ({ ...aiSettings, sttEngine: "auto", sttModel: turbo.id }),
			list_models: () => ({ ...models, stt: [turbo] }),
			get_apple_stt_status: () => ({
				available: true,
				authorized: true,
				supportedLocales: ["en-US", "de-DE"],
				installedLocales: ["en-US"]
			})
		});
		renderCard();
		expect(await screen.findByLabelText("Speech language")).toBeInTheDocument();
		expect(
			screen.getByText(/Used by Apple Speech and by the whisper fallback/)
		).toBeInTheDocument();
	});

	it("explains visibly why Apple Speech is unavailable and exposes the selected engine", async () => {
		mockCard();
		renderCard();
		const apple = await screen.findByRole("button", { name: "Apple Speech" });
		expect(apple).toBeDisabled();
		const hint = screen.getByText(/Apple Speech needs macOS 26 or newer/);
		expect(hint).toBeVisible();
		expect(apple).toHaveAccessibleDescription(hint.textContent);
		expect(screen.getByRole("button", { name: "Whisper" })).toHaveAttribute(
			"aria-pressed",
			"true"
		);
		expect(screen.getByRole("button", { name: "Auto" })).toHaveAttribute(
			"aria-pressed",
			"false"
		);
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

	it("shows engine status from the shared store without refetching on changes", async () => {
		mockCard();
		renderCard();
		await screen.findByText("Whisper tiny (English)");
		const invokesBefore = vi.mocked(invoke).mock.calls.length;

		act(() => {
			$aiStatus.set({
				llm: { state: "error", model_id: "gemma-4-E2B-qat", error: "llm exploded" },
				stt: { state: "loading", model_id: "whisper-tiny-en" }
			});
		});
		expect(await screen.findByText("llm exploded")).toBeInTheDocument();
		expect(screen.getByText("Loading...")).toBeInTheDocument();
		// a status change is not a reason to reload models/settings/disk space
		expect(vi.mocked(invoke).mock.calls.length).toBe(invokesBefore);
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

	it.each(SECRET_CONTROLS)(
		"saving the $name shows its confirmed presence and hint",
		async (control) => {
			const user = userEvent.setup();
			mockSecretBackend(control.secretKey, control.hint);
			renderCard();
			const input = await control.openInput(user);
			await user.type(input, "sk-fake-cred");
			await user.click(
				within(input.parentElement!).getByRole("button", { name: control.saveLabel })
			);
			// presence + hint come from the refreshed settings, and the
			// typed value only leaves the input once the backend confirmed
			expect(
				await screen.findByText(
					`Saved (${control.hint}). It is stored locally and never displayed.`
				)
			).toBeInTheDocument();
			expect(input).toHaveValue("");
		}
	);

	it.each(SECRET_CONTROLS)(
		"removing the $name drops its stored presence and hint",
		async (control) => {
			const user = userEvent.setup();
			mockSecretBackend(control.secretKey, control.hint, { initialStored: true });
			renderCard();
			const input = await control.openInput(user);
			expect(
				screen.getByText(/It is stored locally and never displayed/)
			).toBeInTheDocument();
			await user.click(within(input.parentElement!).getByRole("button", { name: "Remove" }));
			await waitFor(() =>
				expect(
					screen.queryByText(/It is stored locally and never displayed/)
				).not.toBeInTheDocument()
			);
			// the control no longer offers Remove
			expect(
				within(input.parentElement!).queryByRole("button", { name: "Remove" })
			).not.toBeInTheDocument();
		}
	);

	it("a rejected secret save keeps the typed value and stays retryable", async () => {
		const user = userEvent.setup();
		let reject = true;
		mockSecretBackend("hfToken", "••••hf42", {
			failSave: () => (reject ? "keychain locked" : undefined)
		});
		renderCard();
		const input = await screen.findByLabelText(/HuggingFace access token/);
		await user.type(input, "hf_fake");
		const row = input.parentElement!;
		await user.click(within(row).getByRole("button", { name: "Save Token" }));
		// the typed token survives the rejection and the failure is visible
		expect(input).toHaveValue("hf_fake");
		expect(
			await screen.findByText(/the typed value is kept so you can retry/)
		).toBeInTheDocument();
		expect(await screen.findByText("keychain locked")).toBeInTheDocument();
		// the control stays retryable: the same value goes through again
		reject = false;
		await user.click(within(row).getByRole("button", { name: "Save Token" }));
		expect(
			await screen.findByText(/Saved \(••••hf42\). It is stored locally and never displayed./)
		).toBeInTheDocument();
		expect(input).toHaveValue("");
	});

	it("a failed remove still shows the stored secret", async () => {
		const user = userEvent.setup();
		mockSecretBackend("extLlmApiKey", "••••llm99", {
			initialStored: true,
			failSave: () => "keychain locked"
		});
		renderCard();
		const llm = await screen.findByRole("region", { name: /Language model/ });
		await user.click(within(llm).getByText("External LLM endpoint"));
		const input = within(llm).getByLabelText("LLM API key (if needed)");
		expect(screen.getByText(/It is stored locally and never displayed/)).toBeInTheDocument();
		await user.click(within(input.parentElement!).getByRole("button", { name: "Remove" }));
		expect(
			await screen.findByText(/the stored value is unchanged, try again/)
		).toBeInTheDocument();
		expect(await screen.findByText("keychain locked")).toBeInTheDocument();
		// the confirmed presence is untouched by the failed clear
		expect(screen.getByText(/It is stored locally and never displayed/)).toBeInTheDocument();
	});

	it("a failed settings refresh after a confirmed save stays truthful", async () => {
		const user = userEvent.setup();
		// the mount read succeeds; the post-save refresh fails
		mockSecretBackend("hfToken", "••••hf42", { failRefreshAfter: 1 });
		renderCard();
		const input = await screen.findByLabelText(/HuggingFace access token/);
		await user.type(input, "hf_fake");
		await user.click(within(input.parentElement!).getByRole("button", { name: "Save Token" }));
		// the save itself was confirmed: the input is cleared...
		await waitFor(() => expect(input).toHaveValue(""));
		// ...but the presence is not faked from the local mutation...
		expect(
			screen.queryByText(/It is stored locally and never displayed/)
		).not.toBeInTheDocument();
		// ...and the refresh failure is visible
		expect(
			await screen.findByText(/re-reading the settings failed: settings db busy/)
		).toBeInTheDocument();
	});

	it("dirty endpoint edits survive a confirmed secret save", async () => {
		const user = userEvent.setup();
		mockSecretBackend("extSttApiKey", "••••stt7");
		renderCard();
		const stt = await screen.findByRole("region", { name: /Speech-to-text/ });
		await user.type(within(stt).getByLabelText("STT base URL"), "localh");
		await user.click(within(stt).getByText("External STT endpoint"));
		const input = within(stt).getByLabelText("STT API key (if needed)");
		await user.type(input, "sk-fake");
		await user.click(within(input.parentElement!).getByRole("button", { name: "Save" }));
		expect(
			await within(stt).findByText(/It is stored locally and never displayed/)
		).toBeInTheDocument();
		// the unsaved URL edit is still pending in its own section
		expect(within(stt).getByLabelText("STT base URL")).toHaveValue("localh");
		expect(within(stt).getByRole("button", { name: "Save STT endpoint" })).toBeEnabled();
	});

	it("rapid clicks on Remove while the clear is pending submit it only once", async () => {
		const user = userEvent.setup();
		let calls = 0;
		const settle: (() => void)[] = [];
		mockCard({
			get_ai_settings: () => ({
				...aiSettings,
				hfTokenSet: true,
				hfTokenHint: "••••hf42"
			}),
			save_ai_settings: () => {
				calls++;
				return new Promise<void>((resolve) => settle.push(resolve));
			}
		});
		renderCard();
		const input = await screen.findByLabelText(/HuggingFace access token/);
		const remove = within(input.parentElement!).getByRole("button", { name: "Remove" });
		await user.click(remove);
		await user.click(remove);
		expect(calls).toBe(1);
		await act(async () => {
			settle[0]!();
		});
		expect(
			vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "save_ai_settings")
		).toHaveLength(1);
	});

	it("rapid clicks on Save while the secret save is pending submit it only once", async () => {
		const user = userEvent.setup();
		let calls = 0;
		let stored = false;
		const settle: (() => void)[] = [];
		mockCard({
			get_ai_settings: () => ({
				...aiSettings,
				hfTokenSet: stored,
				hfTokenHint: stored ? "••••hf42" : null
			}),
			save_ai_settings: () => {
				calls++;
				stored = true;
				return new Promise<void>((resolve) => settle.push(resolve));
			}
		});
		renderCard();
		const input = await screen.findByLabelText(/HuggingFace access token/);
		await user.type(input, "hf_fake");
		const save = within(input.parentElement!).getByRole("button", { name: "Save Token" });
		await user.click(save);
		await user.click(save);
		expect(calls).toBe(1);
		await act(async () => {
			settle[0]!();
		});
		// the single confirmed save shows up, and nothing re-submits
		expect(
			await screen.findByText(/Saved \(••••hf42\). It is stored locally and never displayed./)
		).toBeInTheDocument();
		expect(
			vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "save_ai_settings")
		).toHaveLength(1);
	});
});

describe("useAiModels.saveSecret", () => {
	it("refreshes the displayed presence and hint after a confirmed save", async () => {
		mockSecretBackend("hfToken", "••••hf42");
		const openSnackbar = vi.fn();
		const { result } = renderHook(() => useAiModels(openSnackbar));
		await waitFor(() => expect(result.current.settings).not.toBeNull());
		let confirmed: boolean | undefined;
		await act(async () => {
			confirmed = await result.current.saveSecret("hfToken", "fake-test-token");
		});
		expect(confirmed).toBe(true);
		await waitFor(() => expect(result.current.savedSettings?.hfTokenSet).toBe(true));
		// the DISPLAYED settings reflect the confirmed save too
		expect(result.current.settings?.hfTokenSet).toBe(true);
		expect(result.current.settings?.hfTokenHint).toBe("••••hf42");
	});

	it("drops presence in both snapshots after a confirmed clear", async () => {
		mockSecretBackend("extLlmApiKey", "••••llm99", { initialStored: true });
		const { result } = renderHook(() => useAiModels(vi.fn()));
		await waitFor(() => expect(result.current.settings?.extLlmApiKeySet).toBe(true));
		let confirmed: boolean | undefined;
		await act(async () => {
			confirmed = await result.current.saveSecret("extLlmApiKey", "");
		});
		expect(confirmed).toBe(true);
		await waitFor(() => expect(result.current.savedSettings?.extLlmApiKeySet).toBe(false));
		expect(result.current.settings?.extLlmApiKeySet).toBe(false);
		expect(result.current.settings?.extLlmApiKeyHint).toBeNull();
	});
});
