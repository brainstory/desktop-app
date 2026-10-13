import { Card } from "./ProfileCards";
import { useAiModels } from "./useAiModels";
import type { ModelListContext } from "./ai-models/ModelRow";
import { localeLabel } from "./ai-models/format";
import { LlmSection } from "./ai-models/LlmSection";
import { SttEngineSection } from "./ai-models/SttEngineSection";
import { ModelDownloadsSection } from "./ai-models/ModelDownloadsSection";

import Button from "@ds/Button";
import ErrorSection from "@components/error/ErrorSection";

interface AiModelsCardProps {
	openSnackbar: (isSuccess: boolean, message: string) => void;
}

export function AiModelsCard({ openSnackbar }: AiModelsCardProps) {
	const {
		models,
		settings: maybeSettings,
		savedSettings,
		runtime,
		downloadProgress,
		downloadRates,
		pausedDownloads,
		downloadFailures,
		rowPending,
		appleStt,
		freeBytes,
		settingsError,
		modelsError,
		refreshModels,
		retrySettings,
		download,
		save,
		saveSecret,
		saveEndpoint,
		setSavedSettings,
		setSettings,
		deleteModel,
		cancelDownload,
		pauseDownload,
		activateModel
	} = useAiModels(openSnackbar);

	if (!maybeSettings) {
		return (
			<Card title="AI Models">
				{settingsError !== null ? (
					// a failed read shows the error + a retry instead of a
					// card that spins on "Loading..." forever
					<ErrorSection
						title="Couldn't load your AI settings"
						paragraphs={[settingsError]}
						action={
							<Button variant="pink" onClick={retrySettings}>
								Try again
							</Button>
						}
						hideDashboardLink
					/>
				) : (
					<p className="text-sm text-stone-500">Loading...</p>
				)}
			</Card>
		);
	}
	const settings = maybeSettings;

	const modelList: ModelListContext = {
		downloadProgress,
		pausedDownloads,
		downloadRates,
		failures: downloadFailures,
		pending: rowPending,
		freeBytes,
		actions: {
			onDownload: download,
			onCancel: cancelDownload,
			onPause: pauseDownload,
			onDelete: deleteModel,
			onActivate: activateModel
		}
	};

	const llmStatus = runtime.llm;
	const sttStatus = runtime.stt;
	// recommendation straight from the catalog, so copy never drifts
	const smallestLlm = models.llm.reduce<(typeof models)["llm"][number] | null>(
		(smallest, m) => (!smallest || m.sizeBytes < smallest.sizeBytes ? m : smallest),
		null
	);
	const needsLlm =
		llmStatus.state === "missing" && settings.llmMode !== "external" && !settings.extLlmBaseUrl;
	// mirrors the backend's effective_stt_engine resolution
	const appleActive =
		settings.sttEngine === "apple" || (settings.sttEngine === "auto" && !!appleStt?.available);
	// mirrors the backend's uses_external_stt: switched on AND a URL saved
	const usingExternalStt = settings.sttMode === "external" && !!settings.extSttBaseUrl;
	const needsStt = sttStatus.state === "missing" && !usingExternalStt && !appleActive;

	const usingExternalLlm = settings.llmMode === "external";
	const activeLlmLabel = usingExternalLlm
		? `External endpoint ${settings.extLlmBaseUrl} (${settings.extLlmModel || "default model"})`
		: (() => {
				const active = models.llm.find((m) => m.active);
				if (active?.downloaded) return `Local ${active.label} (ready)`;
				if (active) return `Local ${active.label} (not downloaded yet)`;
				return "Local model (none selected)";
			})();
	const activeSttLabel = usingExternalStt
		? `External endpoint ${settings.extSttBaseUrl}`
		: appleActive
			? `Apple Speech on-device (${localeLabel(settings.sttLanguage || "en-US")})`
			: (() => {
					const active = models.stt.find((m) => m.active);
					if (active?.downloaded) return `Local ${active.label} (ready)`;
					if (active) return `Local ${active.label} (not downloaded yet)`;
					return "Local model (none selected)";
				})();

	return (
		<Card
			title="AI Models"
			subtitle="Everything runs locally on this machine. You can also offload to an external OpenAI-compatible endpoint."
			columns={3}
		>
			<div className="mb-4 bg-stone-100 border border-stone-200 rounded-lg p-4 text-sm">
				<p className="font-semibold mb-1">What&rsquo;s being used right now</p>
				<p>
					<span className="font-medium">Brainstorming:</span> {activeLlmLabel}
				</p>
				<p>
					<span className="font-medium">Speech-to-text:</span> {activeSttLabel}
				</p>
			</div>
			{settings.secretsPlaintext && (
				<div className="mb-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 text-sm">
					<p className="font-semibold mb-1">Stored keys are not encrypted</p>
					<p>
						No system keychain is in use here, so saved API keys live in the local
						database instead. Anything that can read this computer&rsquo;s files can
						read them.
					</p>
				</div>
			)}
			{needsLlm && (
				<div className="mb-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 text-sm">
					<p className="font-semibold mb-1">No language model is set up yet</p>
					<p>
						Brainstorming needs an AI brain: download one of the models below
						(recommended: {smallestLlm ? smallestLlm.label : "the smallest one"}), or
						set up an external LLM endpoint in the language model section.
					</p>
				</div>
			)}
			{needsStt && (
				<div className="mb-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 text-sm">
					<p className="font-semibold mb-1">No speech-to-text model is set up yet</p>
					<p>
						Download a whisper model below (or set up an external STT endpoint in the
						speech-to-text section) to talk out loud. Until then you can still use
						Brainstory by typing your responses with the text button in a session.
					</p>
				</div>
			)}

			{modelsError && (
				<div
					role="alert"
					className="mb-4 flex items-center justify-between gap-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 text-sm"
				>
					<span>
						Couldn&rsquo;t load the model list — the sections below may be incomplete.
					</span>
					<Button variant="bordered" onClick={refreshModels}>
						Try again
					</Button>
				</div>
			)}

			<LlmSection
				settings={settings}
				savedSettings={savedSettings}
				setSettings={setSettings}
				setSavedSettings={setSavedSettings}
				status={llmStatus}
				models={models.llm}
				modelList={modelList}
				save={save}
				saveSecret={saveSecret}
				refreshModels={refreshModels}
				openSnackbar={openSnackbar}
			/>

			<hr className="my-6 border-stone-200" />

			<SttEngineSection
				settings={settings}
				status={sttStatus}
				models={models.stt}
				modelList={modelList}
				appleStt={appleStt}
				appleActive={appleActive}
				save={save}
				savedSettings={savedSettings}
				setSettings={setSettings}
				setSavedSettings={setSavedSettings}
				saveSecret={saveSecret}
				refreshModels={refreshModels}
				openSnackbar={openSnackbar}
			/>

			<hr className="my-6 border-stone-200" />

			<ModelDownloadsSection
				settings={settings}
				saveSecret={saveSecret}
				saveEndpoint={saveEndpoint}
			/>
		</Card>
	);
}

export default AiModelsCard;
