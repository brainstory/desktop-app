import { useId, useState, type Dispatch, type SetStateAction } from "react";
import { cn } from "@helpers/cn";
import OnOffToggleButton from "@ds/OnOffToggleButton";
import type {
	AiSettingsResponse,
	AppleSttStatus,
	EngineStatus,
	ModelStatus
} from "@helpers/api/models";
import { EngineStatusHeader } from "./EngineStatusHeader";
import { ExternalEndpointFields } from "./ExternalEndpointFields";
import { ModelList, type ModelListContext } from "./ModelRow";
import { localeLabel } from "./format";

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

const ENGINE_OPTIONS = [
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
] as const;

interface SttEngineSectionProps {
	settings: AiSettingsResponse;
	status: EngineStatus;
	models: ModelStatus[];
	modelList: ModelListContext;
	appleStt: AppleSttStatus | null;
	/** Apple Speech is the effective engine (mirrors the backend) */
	appleActive: boolean;
	save: (updates: Partial<AiSettingsResponse>) => void;
	savedSettings: AiSettingsResponse | null;
	setSettings: Dispatch<SetStateAction<AiSettingsResponse | null>>;
	setSavedSettings: Dispatch<SetStateAction<AiSettingsResponse | null>>;
	saveSecret: (key: string, value: string) => void;
	refreshModels: () => void;
	openSnackbar: (isSuccess: boolean, message: string) => void;
}

/** Speech-to-text: status, engine choice, language, whisper models, and
 * its external endpoint. */
export function SttEngineSection({
	settings,
	status,
	models,
	modelList,
	appleStt,
	appleActive,
	save,
	savedSettings,
	setSettings,
	setSavedSettings,
	saveSecret,
	refreshModels,
	openSnackbar
}: SttEngineSectionProps) {
	const headingId = useId();
	const appleHintId = useId();
	const externalLabelId = useId();
	// mirrors the backend's uses_external_stt: switched on AND a URL saved
	const switchedOn = settings.sttMode === "external";
	const usingExternal = switchedOn && !!savedSettings?.extSttBaseUrl;
	// start the endpoint open whenever it is (or is about to be) relevant
	const [endpointOpen, setEndpointOpen] = useState(switchedOn || !!savedSettings?.extSttBaseUrl);

	const toggleExternal = () => {
		const next = switchedOn ? "local" : "external";
		// the switch saves only sttMode, so the URL must already be
		// persisted - a typed-but-unsaved one doesn't count
		if (next === "external" && !savedSettings?.extSttBaseUrl) {
			setEndpointOpen(true);
			openSnackbar(
				false,
				settings.extSttBaseUrl
					? "Save the external STT endpoint URL first, then enable this"
					: "Set an external STT endpoint URL first, then enable this"
			);
			return;
		}
		if (next === "external") setEndpointOpen(true);
		save({ sttMode: next });
	};

	// The language reaches Apple Speech and multilingual whisper models
	// (directly, or as Apple's fallback); English-only builds ignore it.
	// The configured whisper model, not the `active` flag: list_models marks
	// no whisper model active while Apple Speech runs, yet a downloaded one
	// still serves as its fallback.
	const whisperModel = models.find((m) => m.id === settings.sttModel);
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

	return (
		<section aria-labelledby={headingId} className="flex flex-col gap-3">
			<EngineStatusHeader
				title="Speech-to-text (transcribes you)"
				status={status}
				headingId={headingId}
			/>
			<div className="flex gap-2 items-center">
				<span className="text-sm text-stone-600" id={externalLabelId}>
					Use external STT endpoint
				</span>
				<OnOffToggleButton
					aria-labelledby={externalLabelId}
					checked={switchedOn}
					onToggle={toggleExternal}
				/>
			</div>
			{usingExternal && (
				<p className="text-sm bg-stone-100 border border-stone-200 rounded-lg p-3">
					Transcription uses the external STT endpoint below. Turn the switch off to
					transcribe on this computer with the engine chosen here.
				</p>
			)}
			<div className="flex flex-col gap-2 border border-stone-200 rounded-lg p-4">
				<p className="font-semibold text-stone-900">Engine</p>
				<div className="flex flex-wrap gap-2">
					{ENGINE_OPTIONS.map((opt) => {
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
			<ModelList models={models} {...modelList} />
			<details
				open={endpointOpen}
				onToggle={(e) => setEndpointOpen(e.currentTarget.open)}
				className="border border-stone-200 rounded-lg p-4 text-sm"
			>
				<summary className="font-semibold cursor-pointer">External STT endpoint</summary>
				<p className="text-stone-500 mt-2 mb-4">
					Any OpenAI-compatible transcription server (e.g. a whisper.cpp server). Save the
					URL, then turn on the switch above to use it instead of transcribing on this
					computer.
				</p>
				<ExternalEndpointFields
					kind="stt"
					settings={settings}
					savedSettings={savedSettings}
					setSettings={setSettings}
					setSavedSettings={setSavedSettings}
					refreshModels={refreshModels}
					saveSecret={saveSecret}
					openSnackbar={openSnackbar}
				/>
			</details>
		</section>
	);
}
