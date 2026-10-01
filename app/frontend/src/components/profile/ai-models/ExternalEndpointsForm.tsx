import type { Dispatch, SetStateAction } from "react";
import PinkButton from "@ds/PinkButton";
import BorderedButton from "@ds/BorderedButton";
import { normalizeApiError } from "@helpers/helpers";
import {
	saveAiSettingsApi,
	testLlmEndpointApi,
	testSttEndpointApi,
	type AiSettingsResponse
} from "@helpers/api/models";
import { SecretRow } from "./SecretRow";

const ENDPOINT_KEYS = ["extLlmBaseUrl", "extLlmModel", "extSttBaseUrl", "extSttModel"] as const;
type EndpointKey = (typeof ENDPOINT_KEYS)[number];

const ENDPOINT_FIELDS: { key: EndpointKey; label: string; placeholder: string }[] = [
	{ key: "extLlmBaseUrl", label: "LLM base URL", placeholder: "http://localhost:11434" },
	{ key: "extLlmModel", label: "LLM model", placeholder: "llama3.1:8b" },
	{ key: "extSttBaseUrl", label: "STT base URL", placeholder: "http://localhost:8080" },
	{ key: "extSttModel", label: "STT model", placeholder: "whisper-1" }
];

interface ExternalEndpointsFormProps {
	settings: AiSettingsResponse;
	savedSettings: AiSettingsResponse | null;
	setSettings: Dispatch<SetStateAction<AiSettingsResponse | null>>;
	setSavedSettings: Dispatch<SetStateAction<AiSettingsResponse | null>>;
	refreshModels: () => void;
	saveSecret: (key: string, value: string) => void;
	openSnackbar: (isSuccess: boolean, message: string) => void;
}

/**
 * One consistent save model: these text fields save via the button
 * (dirty-gated); the toggles elsewhere save immediately because they're
 * explicit single actions. Test always saves what's typed first, so it
 * can never test a stale endpoint.
 */
export function ExternalEndpointsForm({
	settings,
	savedSettings,
	setSettings,
	setSavedSettings,
	refreshModels,
	saveSecret,
	openSnackbar
}: ExternalEndpointsFormProps) {
	const updateField =
		(key: EndpointKey) =>
		(e: React.ChangeEvent<HTMLInputElement>): void => {
			setSettings({ ...settings, [key]: e.target.value });
		};

	// only the edited fields are sent: the backend validates every key it
	// receives, so re-sending an untouched stale value would fail the
	// whole save
	const changedEndpoints = Object.fromEntries(
		ENDPOINT_KEYS.filter((key) => (settings[key] ?? "") !== (savedSettings?.[key] ?? "")).map(
			(key) => [key, settings[key]]
		)
	) as Partial<AiSettingsResponse>;
	const endpointDirty = Object.keys(changedEndpoints).length > 0;

	const saveEndpoints = (): Promise<void> =>
		saveAiSettingsApi(changedEndpoints)
			.then(() => {
				setSavedSettings((prev) => (prev ? { ...prev, ...changedEndpoints } : prev));
				refreshModels();
				openSnackbar(true, "Settings saved");
			})
			.catch((e: unknown) => {
				openSnackbar(false, normalizeApiError(e));
				throw e;
			});

	const saveAndTest = (testFn: () => Promise<string>): void => {
		const runTest = () =>
			testFn()
				.then((msg) => openSnackbar(true, msg))
				.catch((e) => openSnackbar(false, normalizeApiError(e)));
		if (endpointDirty) {
			saveEndpoints()
				.then(runTest)
				.catch(() => {});
		} else {
			runTest();
		}
	};

	return (
		<div className="flex flex-col gap-4">
			<h3 className="font-semibold">External endpoints (optional)</h3>
			<p className="text-sm text-stone-500 -mt-2">
				Offload AI to any OpenAI-compatible server (Ollama, llama.cpp server, LM Studio,
				...). Base URL example: <code>http://localhost:11434</code>
			</p>
			<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
				{ENDPOINT_FIELDS.map((field) => (
					<div key={field.key}>
						<label
							htmlFor={`endpoint-${field.key}`}
							className="block mb-1 text-sm font-medium text-stone-900"
						>
							{field.label}
						</label>
						<input
							id={`endpoint-${field.key}`}
							type="text"
							value={settings[field.key] ?? ""}
							onChange={updateField(field.key)}
							className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-accent-500 focus:border-accent-500 block w-full p-2"
							placeholder={field.placeholder}
						/>
					</div>
				))}
				<SecretRow
					inputId="ext-llm-api-key"
					label="LLM API key (if needed)"
					stored={settings.extLlmApiKeySet}
					hint={settings.extLlmApiKeyHint}
					placeholder="sk-..."
					onSave={(value) => saveSecret("extLlmApiKey", value)}
				/>
				<SecretRow
					inputId="ext-stt-api-key"
					label="STT API key (if needed)"
					stored={settings.extSttApiKeySet}
					hint={settings.extSttApiKeyHint}
					placeholder="sk-..."
					onSave={(value) => saveSecret("extSttApiKey", value)}
				/>
			</div>
			<div className="flex gap-3">
				<PinkButton
					disabled={!endpointDirty}
					onClick={() => saveEndpoints().catch(() => {})}
				>
					{endpointDirty ? "Save Endpoints" : "All changes saved"}
				</PinkButton>
				<BorderedButton onClick={() => saveAndTest(testLlmEndpointApi)}>
					Test LLM
				</BorderedButton>
				<BorderedButton onClick={() => saveAndTest(testSttEndpointApi)}>
					Test STT
				</BorderedButton>
			</div>
		</div>
	);
}
