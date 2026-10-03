import { useId, useState, type Dispatch, type SetStateAction } from "react";
import OnOffToggleButton from "@ds/OnOffToggleButton";
import type { AiSettingsResponse, EngineStatus, ModelStatus } from "@helpers/api/models";
import { EngineStatusHeader } from "./EngineStatusHeader";
import { ExternalEndpointFields } from "./ExternalEndpointFields";
import { ModelList, type ModelListContext } from "./ModelRow";

/** Context-window choices, in tokens. 0 = the app default (16k). */
const CTX_OPTIONS = [
	{ value: 0, label: "Default (16k)" },
	{ value: 32768, label: "32k" },
	{ value: 65536, label: "64k" },
	{ value: 131072, label: "128k" }
] as const;

/** "~1.6 GB"-style hint for an approximate byte count (1 GB = 1e9 B). */
const approxGb = (bytes: number): string => `~${(bytes / 1e9).toFixed(1)} GB`;

interface LlmSectionProps {
	settings: AiSettingsResponse;
	savedSettings: AiSettingsResponse | null;
	setSettings: Dispatch<SetStateAction<AiSettingsResponse | null>>;
	setSavedSettings: Dispatch<SetStateAction<AiSettingsResponse | null>>;
	status: EngineStatus;
	models: ModelStatus[];
	modelList: ModelListContext;
	save: (updates: Partial<AiSettingsResponse>) => void;
	saveSecret: (key: string, value: string) => Promise<boolean> | void;
	refreshModels: () => void;
	openSnackbar: (isSuccess: boolean, message: string) => void;
}

/** Language model: status, local/external switch, its external endpoint,
 * and the local models. */
export function LlmSection({
	settings,
	savedSettings,
	setSettings,
	setSavedSettings,
	status,
	models,
	modelList,
	save,
	saveSecret,
	refreshModels,
	openSnackbar
}: LlmSectionProps) {
	const headingId = useId();
	const externalLlmLabelId = useId();
	const ctxSelectId = useId();
	const usingExternal = settings.llmMode === "external";
	// the endpoint is where you set things up before switching over, so
	// start it open whenever it is (or is about to be) relevant
	const [endpointOpen, setEndpointOpen] = useState(
		usingExternal || !!savedSettings?.extLlmBaseUrl
	);
	// the memory estimate follows the configured local model (list_models
	// carries each catalog model's approximate KV bytes/token)
	const localModel = models.find((m) => m.id === settings.llmModel);
	const kvPerToken = localModel?.kvBytesPerToken;
	const ctxHelp =
		kvPerToken !== undefined
			? "Approximate extra memory for the model's context (KV cache)."
			: "A bigger window needs more memory; the exact amount depends on the selected model.";

	const toggleExternal = () => {
		const nextMode = usingExternal ? "local" : "external";
		// the toggle saves only llmMode, so the URL must already be
		// persisted - a typed-but-unsaved one doesn't count
		if (nextMode === "external" && !savedSettings?.extLlmBaseUrl) {
			setEndpointOpen(true);
			openSnackbar(
				false,
				settings.extLlmBaseUrl
					? "Save the external endpoint URL first, then enable this"
					: "Set an external endpoint URL first, then enable this"
			);
			return;
		}
		if (nextMode === "external") setEndpointOpen(true);
		save({ llmMode: nextMode });
	};

	return (
		<section aria-labelledby={headingId} className="flex flex-col gap-3">
			<EngineStatusHeader
				title="Language model (brainstorming & writeups)"
				status={status}
				headingId={headingId}
			/>
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
			<details
				open={endpointOpen}
				onToggle={(e) => setEndpointOpen(e.currentTarget.open)}
				className="border border-stone-200 rounded-lg p-4 text-sm"
			>
				<summary className="font-semibold cursor-pointer">External LLM endpoint</summary>
				<p className="text-stone-500 mt-2 mb-4">
					Any OpenAI-compatible server (Ollama, llama.cpp server, LM Studio, ...). Save
					the URL, then turn on the switch above to use it instead of a model on this
					computer.
				</p>
				<ExternalEndpointFields
					kind="llm"
					settings={settings}
					savedSettings={savedSettings}
					setSettings={setSettings}
					setSavedSettings={setSavedSettings}
					refreshModels={refreshModels}
					saveSecret={saveSecret}
					openSnackbar={openSnackbar}
				/>
			</details>
			{!usingExternal && (
				<>
					<h4 className="font-semibold text-stone-700 text-sm">
						Models on this computer
					</h4>
					<ModelList models={models} {...modelList} />
					<div className="max-w-xs">
						<label
							htmlFor={ctxSelectId}
							className="block mb-1 text-sm font-medium text-stone-900"
						>
							Context window
						</label>
						<select
							id={ctxSelectId}
							value={settings.llmCtxTokens}
							onChange={(e) => save({ llmCtxTokens: Number(e.target.value) })}
							className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-accent-500 focus:border-accent-500 block w-full p-2 bg-white"
						>
							{CTX_OPTIONS.map((opt) => (
								<option key={opt.value} value={opt.value}>
									{opt.label}
									{kvPerToken !== undefined
										? ` - ${approxGb(
												(opt.value === 0 ? 16384 : opt.value) * kvPerToken
											)}`
										: ""}
								</option>
							))}
						</select>
						<p className="text-sm text-stone-500 mt-1">
							{ctxHelp} Changing this reloads the model.
						</p>
					</div>
				</>
			)}
		</section>
	);
}
