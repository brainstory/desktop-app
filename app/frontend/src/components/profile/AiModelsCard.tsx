import { Card } from "./ProfileCards";
import PinkButton from "@ds/PinkButton";
import BorderedButton from "@ds/BorderedButton";
import SecretField from "@ds/SecretField";
import OnOffToggleButton from "@ds/OnOffToggleButton";
import { useAiModels } from "./useAiModels";
import { useId, useState } from "react";
import { useConfirmClick } from "@src/hooks/useTimeout";
import { normalizeApiError } from "@helpers/helpers";
import { cn } from "@helpers/cn";
import { testLlmEndpointApi, testSttEndpointApi, saveAiSettingsApi } from "@helpers/api/models";
import type { AiSettingsResponse, ModelStatus } from "@helpers/api/models";

const formatSize = (bytes?: number): string => {
	if (!bytes) return "";
	const gb = bytes / 1_000_000_000;
	if (gb >= 1) return `${gb.toFixed(1)} GB`;
	return `${Math.round(bytes / 1_000_000)} MB`;
};

/**
 * Display percentage for a download: null while the backend could not
 * determine the total size (it reports a negative pct then).
 */
const downloadPercent = (pct: number): number | null =>
	pct < 0 ? null : Math.min(100, Math.floor(pct));

/** Headroom required on top of the model file before we warn about space */
const DOWNLOAD_SPACE_MARGIN_BYTES = 1_000_000_000;

const localeLabel = (id: string): string => {
	try {
		return new Intl.DisplayNames([id], { type: "language" }).of(id) ?? id;
	} catch {
		return id;
	}
};

/**
 * Languages offered for a multilingual whisper model when Apple Speech
 * can't list its locales (whisper uses the primary subtag).
 */
const WHISPER_LOCALES = [
	"en-US",
	"de-DE",
	"es-ES",
	"fr-FR",
	"it-IT",
	"pt-BR",
	"nl-NL",
	"pl-PL",
	"sv-SE",
	"tr-TR",
	"ru-RU",
	"uk-UA",
	"ja-JP",
	"ko-KR",
	"zh-CN",
	"hi-IN",
	"ar-SA"
];

/** Mirrors the backend: `*-en` whisper builds always transcribe English. */
const isMultilingualWhisper = (modelId: string): boolean => !modelId.endsWith("-en");

const ENDPOINT_KEYS = ["extLlmBaseUrl", "extLlmModel", "extSttBaseUrl", "extSttModel"] as const;

const STATUS_LABELS = {
	ready: "Ready",
	loading: "Loading...",
	missing: "Not downloaded",
	error: "Error",
	external: "Using external endpoint"
};

interface AiModelsCardProps {
	openSnackbar: (isSuccess: boolean, message: string) => void;
}

export function AiModelsCard({ openSnackbar }: AiModelsCardProps) {
	const ai = useAiModels(openSnackbar);
	const appleHintId = useId();
	const {
		models,
		settings: maybeSettings,
		savedSettings,
		runtime,
		downloadProgress,
		appleStt,
		freeBytes,
		externalLlmLabelId,
		refreshModels,
		download,
		save,
		saveSecret,
		saveEndpoint,
		setSavedSettings,
		setSettings,
		deleteModel,
		cancelDownload,
		activateModel
	} = ai;

	if (!maybeSettings) {
		return (
			<Card title="AI Models">
				<p className="text-sm text-stone-500">Loading...</p>
			</Card>
		);
	}
	const settings = maybeSettings;

	const updateField =
		(key: "extLlmBaseUrl" | "extLlmModel" | "extSttBaseUrl" | "extSttModel") =>
		(e: React.ChangeEvent<HTMLInputElement>): void => {
			setSettings({ ...settings, [key]: e.target.value });
		};

	const renderModelRow = (model: ModelStatus) => {
		const progress = downloadProgress[model.id];
		const isDownloading = progress !== undefined;
		const pct = progress === undefined ? null : downloadPercent(progress);
		return (
			<div
				key={model.id}
				className="flex flex-col gap-2 border border-stone-200 rounded-lg p-4"
			>
				<div className="flex justify-between items-baseline gap-4">
					<div>
						<p className="font-semibold text-stone-900">
							{model.label}
							{model.active && (
								<span className="ml-2 text-xs font-medium text-pink-600 uppercase">
									Active
								</span>
							)}
						</p>
						<p className="text-sm text-stone-500">{model.description}</p>
					</div>
					<div className="flex gap-2 items-center shrink-0">
						{model.downloaded && !model.active && (
							<BorderedButton onClick={() => activateModel(model.id)}>
								Use
							</BorderedButton>
						)}
						{model.downloaded && !isDownloading && (
							<DeleteModelButton
								isActive={model.active}
								onConfirm={() => deleteModel(model.id)}
							/>
						)}
						{!model.downloaded && !isDownloading && (
							<>
								<PinkButton onClick={() => download(model.id)}>
									Download ({formatSize(model.sizeBytes)})
								</PinkButton>
								{freeBytes !== null && (
									<FreeSpaceNote
										freeBytes={freeBytes}
										needBytes={model.sizeBytes}
									/>
								)}
							</>
						)}
						{isDownloading && (
							<BorderedButton onClick={() => cancelDownload(model.id)}>
								Cancel
							</BorderedButton>
						)}
					</div>
				</div>
				{isDownloading && (
					<div className="flex items-center gap-3">
						<div
							role="progressbar"
							aria-label={`${model.label} download progress`}
							aria-valuemin={0}
							aria-valuemax={100}
							aria-valuenow={pct ?? undefined}
							className="w-full bg-stone-200 rounded-full h-2.5 overflow-hidden"
						>
							{pct === null ? (
								// backend couldn't determine the total size
								<div className="bg-pink-500 h-2.5 w-1/3 rounded-full animate-pulse"></div>
							) : (
								<div
									className="bg-pink-500 h-2.5 rounded-full transition-all"
									style={{ width: `${pct}%` }}
								></div>
							)}
						</div>
						<span className="text-xs text-stone-500 tabular-nums shrink-0 w-10 text-right">
							{pct === null ? "…" : `${pct}%`}
						</span>
					</div>
				)}
			</div>
		);
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
	const needsStt = sttStatus.state === "missing" && !settings.extSttBaseUrl && !appleActive;
	// The language reaches Apple Speech and multilingual whisper models
	// (directly, or as Apple's fallback); English-only builds ignore it.
	// The configured whisper model, not the `active` flag: list_models marks
	// no whisper model active while Apple Speech runs, yet a downloaded one
	// still serves as its fallback.
	const whisperModel = models.stt.find((m) => m.id === settings.sttModel);
	const whisperFallback = appleActive && !!whisperModel?.downloaded;
	const multilingualWhisper =
		!!whisperModel &&
		(!appleActive || whisperFallback) &&
		isMultilingualWhisper(whisperModel.id);
	const showLanguage = appleActive || multilingualWhisper;
	const currentLanguage = settings.sttLanguage || "en-US";
	const appleLocales = appleStt?.supportedLocales ?? [];
	const baseLocales =
		appleLocales.length > 0 ? appleLocales : appleActive ? ["en-US"] : WHISPER_LOCALES;
	const languageOptions = baseLocales.includes(currentLanguage)
		? baseLocales
		: [currentLanguage, ...baseLocales];
	const languageHelp = !appleActive
		? `${whisperModel?.label ?? "The whisper model"} transcribes in this language.`
		: multilingualWhisper
			? "Used by Apple Speech and by the whisper fallback. Missing languages are fetched by macOS on first use."
			: whisperFallback
				? "Used by Apple Speech; missing languages are fetched by macOS on first use. The English-only whisper fallback always transcribes English."
				: "Used by Apple Speech. Missing languages are fetched by macOS on first use.";

	const usingExternalLlm = settings.llmMode === "external";
	const usingExternalStt = !!settings.extSttBaseUrl;
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
			{needsLlm && (
				<div className="mb-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 text-sm">
					<p className="font-semibold mb-1">No language model is set up yet</p>
					<p>
						Brainstorming needs an AI brain: download one of the models below
						(recommended: {smallestLlm ? smallestLlm.label : "the smallest one"}), or
						point at an external endpoint at the bottom of this page.
					</p>
				</div>
			)}
			{needsStt && (
				<div className="mb-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 text-sm">
					<p className="font-semibold mb-1">No speech-to-text model is set up yet</p>
					<p>
						Download a whisper model below (or configure an external STT endpoint) to
						talk out loud. Until then you can still use Brainstory by typing your
						responses with the text button in a session.
					</p>
				</div>
			)}
			<div className="flex flex-col gap-3">
				<div className="flex justify-between items-center">
					<h3 className="font-semibold">Language model (brainstorming & writeups)</h3>
					<span
						className={`text-xs font-medium uppercase rounded-full px-2 py-1 ${
							llmStatus.state === "ready"
								? "bg-green-100 text-green-700"
								: llmStatus.state === "error"
									? "bg-red-100 text-red-700"
									: "bg-stone-100 text-stone-600"
						}`}
					>
						{STATUS_LABELS[llmStatus.state as keyof typeof STATUS_LABELS] ??
							llmStatus.state}
					</span>
				</div>
				{llmStatus.error && <p className="text-sm text-red-600">{llmStatus.error}</p>}
				<div className="flex gap-2 items-center">
					<span className="text-sm text-stone-600" id={externalLlmLabelId}>
						Use external LLM endpoint
					</span>
					<OnOffToggleButton
						aria-labelledby={externalLlmLabelId}
						checked={settings.llmMode === "external"}
						onToggle={() => {
							const nextMode = settings.llmMode === "external" ? "local" : "external";
							// the toggle saves only llmMode, so the URL must already
							// be persisted - a typed-but-unsaved one doesn't count
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
						}}
					/>
				</div>
				{settings.llmMode !== "external" && models.llm.map(renderModelRow)}
				{settings.llmMode !== "external" && (
					<div className="border border-stone-200 rounded-lg p-4 text-sm">
						<label htmlFor="hf-token-input" className="font-semibold mb-1 block">
							HuggingFace access token (optional)
						</label>
						<p className="text-stone-500 mb-2">
							Authenticated downloads are faster and never hit HuggingFace&rsquo;s
							anonymous rate limits. Create a free read token at
							huggingface.co/settings/tokens.
						</p>
						<SecretField
							inputId="hf-token-input"
							placeholder="hf_..."
							stored={settings.hfTokenSet}
							hint={settings.hfTokenHint}
							saveLabel="Save Token"
							onSave={(value) => saveSecret("hfToken", value)}
						/>
						<label
							htmlFor="hf-endpoint-input"
							className="font-semibold mb-1 mt-4 block"
						>
							HuggingFace download endpoint (mirror, optional)
						</label>
						<p className="text-stone-500 mb-2">
							Leave empty to download from huggingface.co directly, or point at a
							mirror (e.g. https://hf-mirror.com). Also picks up the HF_ENDPOINT
							environment variable when launched from a terminal.
						</p>
						<EndpointField
							inputId="hf-endpoint-input"
							initial={settings.hfEndpoint}
							onSave={saveEndpoint}
						/>
					</div>
				)}
			</div>

			<hr className="my-6 border-stone-200" />

			<div className="flex flex-col gap-3">
				<div className="flex justify-between items-center">
					<h3 className="font-semibold">Speech-to-text (transcribes you)</h3>
					<span
						className={`text-xs font-medium uppercase rounded-full px-2 py-1 ${
							sttStatus.state === "ready"
								? "bg-green-100 text-green-700"
								: sttStatus.state === "error"
									? "bg-red-100 text-red-700"
									: "bg-stone-100 text-stone-600"
						}`}
					>
						{STATUS_LABELS[sttStatus.state as keyof typeof STATUS_LABELS] ??
							sttStatus.state}
					</span>
				</div>
				{sttStatus.error && <p className="text-sm text-red-600">{sttStatus.error}</p>}
				<div className="flex flex-col gap-2 border border-stone-200 rounded-lg p-4">
					<p className="font-semibold text-stone-900">Engine</p>
					<div className="flex flex-wrap gap-2">
						{(
							[
								{
									id: "auto",
									label: "Auto",
									hint: "Apple Speech where available, whisper otherwise"
								},
								{
									id: "apple",
									label: "Apple Speech",
									hint: "Built into macOS 26+ - no model download"
								},
								{
									id: "whisper",
									label: "Whisper",
									hint: "Downloaded whisper model"
								}
							] as const
						).map((opt) => {
							const selected = settings.sttEngine === opt.id;
							const disabled = opt.id === "apple" && !appleStt?.available;
							return (
								<button
									key={opt.id}
									type="button"
									title={disabled ? undefined : opt.hint}
									aria-pressed={selected}
									aria-describedby={disabled ? appleHintId : undefined}
									disabled={disabled}
									onClick={() => save({ sttEngine: opt.id })}
									className={cn(
										"text-sm rounded-lg px-3 py-2 border transition-colors",
										selected
											? "bg-accent-600 border-accent-600 text-white hover:bg-accent-700"
											: disabled
												? "border-stone-200 text-stone-300 cursor-not-allowed"
												: "border-stone-300 text-stone-700 hover:border-accent-400"
									)}
								>
									{opt.label}
								</button>
							);
						})}
					</div>
					{appleStt && !appleStt.available && (
						// visible, not just a tooltip on the disabled chip
						<p id={appleHintId} className="text-sm text-stone-500">
							Apple Speech needs macOS 26 or newer and isn&rsquo;t available on this
							system.
						</p>
					)}
					{settings.sttEngine === "auto" && (
						<p className="text-sm text-stone-500">
							{appleStt?.available
								? "Apple Speech is available on this Mac and will be used; whisper is the automatic fallback."
								: "Auto uses whisper for transcription here."}
						</p>
					)}
					{appleActive && appleStt && !appleStt.authorized && (
						<p className="text-sm text-amber-700">
							Apple Speech needs permission once: the next time you record, allow
							Brainstory under System Settings &gt; Privacy &amp; Security &gt; Speech
							Recognition.
						</p>
					)}
					{showLanguage && (
						<div className="mt-1 max-w-xs">
							<label
								htmlFor="stt-language-select"
								className="block mb-1 text-sm font-medium text-stone-900"
							>
								Speech language
							</label>
							<select
								id="stt-language-select"
								value={currentLanguage}
								onChange={(e) => save({ sttLanguage: e.target.value })}
								className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-accent-500 focus:border-accent-500 block w-full p-2 bg-white"
							>
								{languageOptions.map((loc) => (
									<option key={loc} value={loc}>
										{localeLabel(loc)} ({loc})
										{!appleActive || appleStt?.installedLocales.includes(loc)
											? ""
											: " - not installed yet"}
									</option>
								))}
							</select>
							<p className="text-sm text-stone-500 mt-1">{languageHelp}</p>
						</div>
					)}
				</div>
				<h4 className="font-semibold text-stone-700 text-sm">
					{appleActive ? "Whisper model (fallback)" : "Whisper model"}
				</h4>
				{models.stt.map(renderModelRow)}
			</div>

			<hr className="my-6 border-stone-200" />

			<div className="flex flex-col gap-4">
				<h3 className="font-semibold">External endpoints (optional)</h3>
				<p className="text-sm text-stone-500 -mt-2">
					Offload AI to any OpenAI-compatible server (Ollama, llama.cpp server, LM Studio,
					...). Base URL example: <code>http://localhost:11434</code>
				</p>
				{/* One consistent save model: these text fields save via the button
			    (dirty-gated); the toggles above save immediately because they're
			    explicit single actions. Test always saves what's typed first, so
			    it can never test a stale endpoint. */}
				{(() => {
					// only the edited fields are sent: the backend validates every
					// key it receives, so re-sending an untouched stale value
					// would fail the whole save
					const changedEndpoints = Object.fromEntries(
						ENDPOINT_KEYS.filter(
							(key) => (settings[key] ?? "") !== (savedSettings?.[key] ?? "")
						).map((key) => [key, settings[key]])
					) as Partial<AiSettingsResponse>;
					const endpointDirty = Object.keys(changedEndpoints).length > 0;

					const saveEndpoints = (): Promise<void> =>
						saveAiSettingsApi(changedEndpoints)
							.then(() => {
								setSavedSettings((prev) =>
									prev ? { ...prev, ...changedEndpoints } : prev
								);
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

					const endpointFields: {
						key: "extLlmBaseUrl" | "extLlmModel" | "extSttBaseUrl" | "extSttModel";
						label: string;
						placeholder: string;
					}[] = [
						{
							key: "extLlmBaseUrl",
							label: "LLM base URL",
							placeholder: "http://localhost:11434"
						},
						{ key: "extLlmModel", label: "LLM model", placeholder: "llama3.1:8b" },
						{
							key: "extSttBaseUrl",
							label: "STT base URL",
							placeholder: "http://localhost:8080"
						},
						{ key: "extSttModel", label: "STT model", placeholder: "whisper-1" }
					];

					return (
						<>
							<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
								{endpointFields.map((field) => (
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
								<div>
									<label
										htmlFor="ext-llm-api-key"
										className="block mb-1 text-sm font-medium text-stone-900"
									>
										LLM API key (if needed)
									</label>
									<SecretField
										inputId="ext-llm-api-key"
										stored={settings.extLlmApiKeySet}
										hint={settings.extLlmApiKeyHint}
										placeholder="sk-..."
										onSave={(value) => saveSecret("extLlmApiKey", value)}
									/>
								</div>
								<div>
									<label
										htmlFor="ext-stt-api-key"
										className="block mb-1 text-sm font-medium text-stone-900"
									>
										STT API key (if needed)
									</label>
									<SecretField
										inputId="ext-stt-api-key"
										stored={settings.extSttApiKeySet}
										hint={settings.extSttApiKeyHint}
										placeholder="sk-..."
										onSave={(value) => saveSecret("extSttApiKey", value)}
									/>
								</div>
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
						</>
					);
				})()}
			</div>
		</Card>
	);
}

/**
 * Two-click delete with a visible countdown (the shared confirm pattern,
 * as on draft cards). The active model gets a stronger warning.
 */
function DeleteModelButton({ isActive, onConfirm }: { isActive: boolean; onConfirm: () => void }) {
	const { isConfirming, secondsLeft, confirm } = useConfirmClick();
	return (
		<BorderedButton
			onClick={() => {
				if (confirm()) onConfirm();
			}}
			classes={isConfirming ? "border-red-400 text-red-600 whitespace-nowrap" : ""}
		>
			{isConfirming ? (
				<span aria-live="polite">
					{`${isActive ? "Really delete the ACTIVE model?" : "Really delete?"} (${secondsLeft ?? 0}s)`}
				</span>
			) : (
				"Delete"
			)}
		</BorderedButton>
	);
}

/**
 * Warn-only disk space note next to a Download button: muted when there's
 * enough room, amber when free space is below the model size plus a
 * margin. Never blocks the download - running out mid-transfer fails
 * cleanly through the normal verification path.
 */
function FreeSpaceNote({ freeBytes, needBytes }: { freeBytes: number; needBytes?: number }) {
	if (!needBytes) return null;
	const isLow = freeBytes < needBytes + DOWNLOAD_SPACE_MARGIN_BYTES;
	if (isLow) {
		return (
			<span className="text-xs font-medium text-amber-700 whitespace-nowrap">
				Only {formatSize(freeBytes)} free — needs {formatSize(needBytes)}
			</span>
		);
	}
	return (
		<span className="text-xs text-stone-500 whitespace-nowrap">
			{formatSize(freeBytes)} free
		</span>
	);
}

export default AiModelsCard;

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
			<PinkButton disabled={value === (initial ?? "")} onClick={() => onSave(value)}>
				Save
			</PinkButton>
		</div>
	);
}
