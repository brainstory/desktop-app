import { useId, useState } from "react";
import Button from "@ds/Button";
import OnOffToggleButton from "@ds/OnOffToggleButton";
import type { AiSettingsResponse, EngineStatus, ModelStatus } from "@helpers/api/models";
import { EngineStatusHeader } from "./EngineStatusHeader";
import { ModelList, type ModelListContext } from "./ModelRow";
import { SecretRow } from "./SecretRow";

interface LlmSectionProps {
	settings: AiSettingsResponse;
	savedSettings: AiSettingsResponse | null;
	status: EngineStatus;
	models: ModelStatus[];
	modelList: ModelListContext;
	save: (updates: Partial<AiSettingsResponse>) => void;
	saveSecret: (key: string, value: string) => void;
	saveEndpoint: (value: string) => void;
	openSnackbar: (isSuccess: boolean, message: string) => void;
}

/** Language model: status, local/external switch, models, download settings. */
export function LlmSection({
	settings,
	savedSettings,
	status,
	models,
	modelList,
	save,
	saveSecret,
	saveEndpoint,
	openSnackbar
}: LlmSectionProps) {
	const externalLlmLabelId = useId();
	const usingExternal = settings.llmMode === "external";

	const toggleExternal = () => {
		const nextMode = usingExternal ? "local" : "external";
		// the toggle saves only llmMode, so the URL must already be
		// persisted - a typed-but-unsaved one doesn't count
		if (nextMode === "external" && !savedSettings?.extLlmBaseUrl) {
			openSnackbar(
				false,
				settings.extLlmBaseUrl
					? "Save the external endpoint URL first, then enable this"
					: "Set an external endpoint URL first, then enable this"
			);
			return;
		}
		save({ llmMode: nextMode });
	};

	return (
		<div className="flex flex-col gap-3">
			<EngineStatusHeader title="Language model (brainstorming & writeups)" status={status} />
			<div className="flex gap-2 items-center">
				<span className="text-sm text-stone-600" id={externalLlmLabelId}>
					Use external LLM endpoint
				</span>
				<OnOffToggleButton
					aria-labelledby={externalLlmLabelId}
					checked={usingExternal}
					onToggle={toggleExternal}
				/>
			</div>
			{!usingExternal && <ModelList models={models} {...modelList} />}
			{!usingExternal && (
				<div className="border border-stone-200 rounded-lg p-4 text-sm">
					<SecretRow
						inputId="hf-token-input"
						label="HuggingFace access token (optional)"
						labelClassName="font-semibold mb-1 block"
						description={
							<>
								Authenticated downloads are faster and never hit HuggingFace&rsquo;s
								anonymous rate limits. Create a free read token at
								huggingface.co/settings/tokens.
							</>
						}
						placeholder="hf_..."
						stored={settings.hfTokenSet}
						hint={settings.hfTokenHint}
						saveLabel="Save Token"
						onSave={(value) => saveSecret("hfToken", value)}
					/>
					<label htmlFor="hf-endpoint-input" className="font-semibold mb-1 mt-4 block">
						HuggingFace download endpoint (mirror, optional)
					</label>
					<p className="text-stone-500 mb-2">
						Leave empty to download from huggingface.co directly, or point at a mirror
						(e.g. https://hf-mirror.com). Also picks up the HF_ENDPOINT environment
						variable when launched from a terminal.
					</p>
					<EndpointField
						inputId="hf-endpoint-input"
						initial={settings.hfEndpoint}
						onSave={saveEndpoint}
					/>
				</div>
			)}
		</div>
	);
}

/** Plain-text field with its own Save button for a single non-secret
 * setting (the download endpoint/mirror). Sends only its own value. */
function EndpointField({
	inputId,
	initial,
	onSave
}: {
	inputId: string;
	initial?: string;
	onSave: (value: string) => void;
}) {
	const [value, setValue] = useState(initial ?? "");
	return (
		<div className="flex flex-wrap gap-2">
			<input
				id={inputId}
				type="url"
				value={value}
				onChange={(e) => setValue(e.target.value)}
				placeholder="https://huggingface.co"
				className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-accent-500 focus:border-accent-500 flex-1 min-w-0 p-2"
			/>
			<Button
				variant="pink"
				disabled={value === (initial ?? "")}
				onClick={() => onSave(value)}
			>
				Save
			</Button>
		</div>
	);
}
