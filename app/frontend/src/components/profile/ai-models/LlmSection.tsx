import { useId, useState, type Dispatch, type SetStateAction } from "react";
import OnOffToggleButton from "@ds/OnOffToggleButton";
import type { AiSettingsResponse, EngineStatus, ModelStatus } from "@helpers/api/models";
import { EngineStatusHeader } from "./EngineStatusHeader";
import { ExternalEndpointFields } from "./ExternalEndpointFields";
import { ModelList, type ModelListContext } from "./ModelRow";

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
	const usingExternal = settings.llmMode === "external";
	// the endpoint is where you set things up before switching over, so
	// start it open whenever it is (or is about to be) relevant
	const [endpointOpen, setEndpointOpen] = useState(
		usingExternal || !!savedSettings?.extLlmBaseUrl
	);

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
				</>
			)}
		</section>
	);
}
