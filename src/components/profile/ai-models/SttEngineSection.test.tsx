import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { SttEngineSection } from "./SttEngineSection";
import type { AiSettingsResponse, AppleSttStatus, ModelStatus } from "@helpers/api/models";
import type { ModelListContext } from "./ModelRow";

const settings: AiSettingsResponse = {
	llmMode: "local",
	llmModel: "gemma-4-E2B-qat",
	llmCtxTokens: 0,
	extLlmCtxTokens: 0,
	sttModel: "whisper-base-en",
	sttMode: "local",
	sttEngine: "whisper",
	sttLanguage: "en-US",
	hfTokenSet: false,
	hfTokenHint: null,
	secretsPlaintext: false,
	hfEndpoint: "",
	extLlmBaseUrl: "",
	extLlmApiKeySet: false,
	extLlmApiKeyHint: null,
	extLlmModel: "",
	extSttBaseUrl: "",
	extSttApiKeySet: false,
	extSttApiKeyHint: null,
	extSttModel: ""
};

const whisperBase: ModelStatus = {
	id: "whisper-base-en",
	label: "Whisper base (English)",
	description: "Good everyday transcription",
	kind: "stt",
	sizeBytes: 147_964_211,
	downloaded: true,
	active: true,
	downloading: false,
	filename: "ggml-base.en.bin"
};

const appleUnavailable: AppleSttStatus = {
	available: false,
	authorized: false,
	supportedLocales: [],
	installedLocales: []
};

const appleAvailable: AppleSttStatus = {
	available: true,
	authorized: true,
	supportedLocales: ["en-US", "de-DE"],
	installedLocales: ["en-US"]
};

function renderSection(
	overrides: {
		settings?: Partial<AiSettingsResponse>;
		appleStt?: AppleSttStatus | null;
		appleActive?: boolean;
		models?: ModelStatus[];
	} = {}
) {
	const save = vi.fn();
	const modelList: ModelListContext = {
		downloadProgress: {},
		pausedDownloads: {},
		downloadRates: {},
		failures: {},
		pending: {},
		freeBytes: null,
		actions: {
			onDownload: vi.fn(),
			onCancel: vi.fn(),
			onPause: vi.fn(),
			onDelete: vi.fn(),
			onActivate: vi.fn()
		}
	};
	render(
		<SttEngineSection
			settings={{ ...settings, ...overrides.settings }}
			status={{ state: "ready", model_id: "whisper-base-en" }}
			models={overrides.models ?? [whisperBase]}
			modelList={modelList}
			appleStt={overrides.appleStt ?? appleUnavailable}
			appleActive={overrides.appleActive ?? false}
			save={save}
			savedSettings={settings}
			setSettings={vi.fn()}
			setSavedSettings={vi.fn()}
			saveSecret={vi.fn()}
			refreshModels={vi.fn()}
			openSnackbar={vi.fn()}
		/>
	);
	return { save };
}

describe("SttEngineSection", () => {
	it("selects an engine through save()", async () => {
		const user = userEvent.setup();
		const { save } = renderSection();
		await user.click(screen.getByRole("button", { name: "Auto" }));
		expect(save).toHaveBeenCalledWith({ sttEngine: "auto" });
	});

	it("disables Apple Speech with a visible reason when unavailable", () => {
		renderSection({ appleStt: appleUnavailable });
		expect(screen.getByRole("button", { name: "Apple Speech" })).toBeDisabled();
		expect(screen.getByText(/needs macOS 26 or newer/)).toBeInTheDocument();
	});

	it("explains what Auto resolves to when Apple Speech exists", () => {
		renderSection({ settings: { sttEngine: "auto" }, appleStt: appleAvailable });
		expect(screen.getByText(/Apple Speech is available on this Mac/)).toBeInTheDocument();
	});

	it("explains what Auto resolves to without Apple Speech", () => {
		renderSection({ settings: { sttEngine: "auto" }, appleStt: appleUnavailable });
		expect(screen.getByText(/Auto uses whisper for transcription here/)).toBeInTheDocument();
	});

	it("hides the speech language for an English-only whisper model", () => {
		renderSection();
		expect(screen.queryByLabelText("Speech language")).not.toBeInTheDocument();
	});

	it("warns when a non-English language meets an English-only model", () => {
		renderSection({
			settings: { sttModel: "whisper-base-en", sttLanguage: "de-DE" }
		});
		expect(screen.getByText(/selected whisper model is English-only/i)).toBeInTheDocument();
	});

	it("offers the speech language for a multilingual whisper model", () => {
		renderSection({
			settings: { sttModel: "whisper-large-v3-turbo" },
			models: [
				{
					...whisperBase,
					id: "whisper-large-v3-turbo",
					label: "Whisper large v3 turbo",
					active: false
				}
			]
		});
		expect(screen.getByLabelText("Speech language")).toHaveValue("en-US");
		expect(
			screen.getByText(/Whisper large v3 turbo transcribes in this language/)
		).toBeInTheDocument();
	});
});
