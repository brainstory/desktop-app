import type { Dispatch, SetStateAction } from "react";
import Button from "@ds/Button";
import { normalizeApiError } from "@helpers/helpers";
import {
	saveAiSettingsApi,
	testLlmEndpointApi,
	testSttEndpointApi,
	type AiSettingsResponse
} from "@helpers/api/models";
import { SecretRow } from "./SecretRow";

/** Everything that differs between the LLM and the STT endpoint. */
const KINDS = {
	llm: {
		name: "LLM",
		urlKey: "extLlmBaseUrl",
		modelKey: "extLlmModel",
		secretKey: "extLlmApiKey",
		secretSet: "extLlmApiKeySet",
		secretHint: "extLlmApiKeyHint",
		urlPlaceholder: "http://localhost:11434",
		modelPlaceholder: "llama3.1:8b",
		test: testLlmEndpointApi
	},
	stt: {
		name: "STT",
		urlKey: "extSttBaseUrl",
		modelKey: "extSttModel",
		secretKey: "extSttApiKey",
		secretSet: "extSttApiKeySet",
		secretHint: "extSttApiKeyHint",
		urlPlaceholder: "http://localhost:8080",
		modelPlaceholder: "whisper-1",
		test: testSttEndpointApi
	}
} as const;

export type EndpointKind = keyof typeof KINDS;

interface ExternalEndpointFieldsProps {
	kind: EndpointKind;
	settings: AiSettingsResponse;
	savedSettings: AiSettingsResponse | null;
	setSettings: Dispatch<SetStateAction<AiSettingsResponse | null>>;
	setSavedSettings: Dispatch<SetStateAction<AiSettingsResponse | null>>;
	refreshModels: () => void;
	saveSecret: (key: string, value: string) => void;
	openSnackbar: (isSuccess: boolean, message: string) => void;
}

/**
 * One OpenAI-compatible endpoint (URL, model, API key) with its own Save
 * and Test. Each section owns its endpoint, so saving the LLM endpoint
 * never sends half-typed STT fields and vice versa. The text fields save
 * via the button (dirty-gated); Test saves what's typed first, so it can
 * never test a stale endpoint. The API key saves on its own (secrets are
 * never echoed back to the form).
 */
export function ExternalEndpointFields({
	kind,
	settings,
	savedSettings,
	setSettings,
	setSavedSettings,
	refreshModels,
	saveSecret,
	openSnackbar
}: ExternalEndpointFieldsProps) {
	const k = KINDS[kind];
	const fieldKeys = [k.urlKey, k.modelKey] as const;
	const fields = [
		{ key: k.urlKey, label: `${k.name} base URL`, placeholder: k.urlPlaceholder },
		{ key: k.modelKey, label: `${k.name} model`, placeholder: k.modelPlaceholder }
	];

	// only this endpoint's edited fields are sent: the backend validates
	// every key it receives, so re-sending an untouched stale value would
	// fail the whole save
	const changed = Object.fromEntries(
		fieldKeys
			.filter((key) => (settings[key] ?? "") !== (savedSettings?.[key] ?? ""))
			.map((key) => [key, settings[key]])
	) as Partial<AiSettingsResponse>;
	const dirty = Object.keys(changed).length > 0;

	const save = (): Promise<void> =>
		saveAiSettingsApi(changed)
			.then(() => {
				setSavedSettings((prev) => (prev ? { ...prev, ...changed } : prev));
				refreshModels();
				openSnackbar(true, `${k.name} endpoint saved`);
			})
			.catch((e: unknown) => {
				openSnackbar(false, normalizeApiError(e));
				throw e;
			});

	const saveAndTest = (): void => {
		const runTest = () =>
			k
				.test()
				.then((msg) => openSnackbar(true, msg))
				.catch((e) => openSnackbar(false, normalizeApiError(e)));
		if (dirty) {
			save()
				.then(runTest)
				.catch(() => {});
		} else {
			// runTest reports its own failure
			void runTest();
		}
	};

	return (
		<div className="flex flex-col gap-4">
			<div className="grid grid-cols-1 md:grid-cols-2 gap-4">
				{fields.map((field) => (
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
							onChange={(e) =>
								setSettings({ ...settings, [field.key]: e.target.value })
							}
							className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-accent-500 focus:border-accent-500 block w-full p-2"
							placeholder={field.placeholder}
						/>
					</div>
				))}
			</div>
			<div className="flex flex-wrap gap-3">
				<Button
					variant="pink"
					disabled={!dirty}
					onClick={() => void save().catch(() => {})}
				>
					{dirty ? `Save ${k.name} endpoint` : "Endpoint saved"}
				</Button>
				<Button variant="bordered" onClick={saveAndTest}>
					Test {k.name} endpoint
				</Button>
			</div>
			<SecretRow
				inputId={`ext-${kind}-api-key`}
				label={`${k.name} API key (if needed)`}
				stored={settings[k.secretSet]}
				hint={settings[k.secretHint]}
				placeholder="sk-..."
				onSave={(value) => saveSecret(k.secretKey, value)}
			/>
		</div>
	);
}
