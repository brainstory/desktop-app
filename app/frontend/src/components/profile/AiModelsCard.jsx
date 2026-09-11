import { Card } from "./ProfileCards";
import PinkButton from "@ds/PinkButton";
import BorderedButton from "@ds/BorderedButton";
import OnOffToggleButton from "@ds/OnOffToggleButton";
import { useState } from "react";
import { useEffect } from "react";

import {
	listModelsApi,
	getAiSettingsApi,
	saveAiSettingsApi,
	downloadModelApi,
	deleteModelApi,
	activateModelApi,
	testLlmEndpointApi,
	testSttEndpointApi,
	getRuntimeStatusApi
} from "@helpers/api/models";
import { listen } from "@tauri-apps/api/event";

const formatSize = (bytes) => {
	if (!bytes) return "";
	const gb = bytes / 1_000_000_000;
	if (gb >= 1) return `${gb.toFixed(1)} GB`;
	return `${Math.round(bytes / 1_000_000)} MB`;
};

const STATUS_LABELS = {
	ready: "Ready",
	loading: "Loading...",
	missing: "Not downloaded",
	error: "Error",
	external: "Using external endpoint"
};

export function AiModelsCard({ openSnackbar }) {
	const [models, setModels] = useState({ llm: [], stt: [] });
	const [settings, setSettings] = useState(null);
	const [runtime, setRuntime] = useState({ llm: {}, stt: {} });
	const [downloadProgress, setDownloadProgress] = useState({});

	const refresh = () => {
		listModelsApi()
			.then((data) => {
				setModels(data);
				// rehydrate in-flight downloads (e.g. after navigating away and back)
				setDownloadProgress((prev) => {
					const next = {};
					for (const list of [data.llm, data.stt]) {
						for (const model of list) {
							if (model.downloading) {
								next[model.id] = model.progress ?? 0;
							}
						}
					}
					return next;
				});
			})
			.catch((e) => console.log("list models failed", e));
		getRuntimeStatusApi().then(setRuntime).catch((e) => console.log("status failed", e));
	};

	useEffect(() => {
		getAiSettingsApi().then(setSettings).catch((e) => console.log("ai settings failed", e));
		refresh();

		const unlisteners = [
			listen("llm-status", refresh),
			listen("stt-status", refresh),
			// downloads continue in the backend across page navigation
			listen("model-download", (event) => {
				const { modelId, kind, pct, message } = event.payload ?? {};
				if (kind === "progress") {
					setDownloadProgress((prev) => ({ ...prev, [modelId]: pct }));
				} else if (kind === "done") {
					setDownloadProgress((prev) => {
						const next = { ...prev };
						delete next[modelId];
						return next;
					});
					openSnackbar(true, "Model downloaded");
					refresh();
				} else if (kind === "error") {
					setDownloadProgress((prev) => {
						const next = { ...prev };
						delete next[modelId];
						return next;
					});
					openSnackbar(false, `Download failed: ${message}`);
				}
			})
		];
		return () => {
			unlisteners.forEach((p) => p.then((fn) => fn()));
		};
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, []);

	if (!settings) {
		return (
			<Card title="AI Models">
				<p className="text-sm text-stone-500">Loading...</p>
			</Card>
		);
	}

	const download = (modelId) => {
		downloadModelApi(modelId).invokePromise.catch((e) => {
			// starting a download that's already running is harmless - the
			// progress bar is already driven by backend events
			if (!String(e).includes("already downloading")) {
				openSnackbar(false, e);
			}
			refresh();
		});
	};

	const save = (updates) => {
		const next = { ...settings, ...updates };
		setSettings(next);
		saveAiSettingsApi(next)
			.then(() => refresh())
			.catch((e) => openSnackbar(false, e));
	};

	const updateField = (key) => (e) => {
		setSettings({ ...settings, [key]: e.target.value });
	};

	const renderModelRow = (model) => {
		const isDownloading = downloadProgress[model.id] !== undefined;
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
							<BorderedButton onClick={() => activateModelApi(model.id).then(refresh).catch((e) => openSnackbar(false, e))}>
								Use
							</BorderedButton>
						)}
						{model.downloaded && !isDownloading && (
							<BorderedButton onClick={() => deleteModelApi(model.id).then(refresh).catch((e) => openSnackbar(false, e))}>
								Delete
							</BorderedButton>
						)}
						{!model.downloaded && !isDownloading && (
							<PinkButton onClick={() => download(model.id)}>
								Download ({formatSize(model.sizeBytes)})
							</PinkButton>
						)}
					</div>
				</div>
				{isDownloading && (
					<div className="w-full bg-stone-200 rounded-full h-2.5">
						<div
							className="bg-pink-500 h-2.5 rounded-full transition-all"
							style={{ width: `${downloadProgress[model.id] ?? 0}%` }}
						></div>
					</div>
				)}
			</div>
		);
	};

	const llmStatus = runtime.llm ?? {};
	const sttStatus = runtime.stt ?? {};
	const needsLlm =
		llmStatus.state === "missing" &&
		settings.llmMode !== "external" &&
		!settings.extLlmBaseUrl;
	const needsStt = sttStatus.state === "missing" && !settings.extSttBaseUrl;

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
				<p className="font-semibold mb-1">What's being used right now</p>
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
						(recommended: the light Gemma 4), or point at an external endpoint at
						the bottom of this page.
					</p>
				</div>
			)}
			{needsStt && (
				<div className="mb-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 text-sm">
					<p className="font-semibold mb-1">No speech-to-text model is set up yet</p>
					<p>
						Download a whisper model below (or configure an external STT endpoint)
						to talk out loud. Until then you can still use Brainstory by typing
						your responses with the text button in a session.
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
						{STATUS_LABELS[llmStatus.state] ?? llmStatus.state}
					</span>
				</div>
				{llmStatus.error && (
					<p className="text-sm text-red-600">{llmStatus.error}</p>
				)}
				<div className="flex gap-2 items-center">
					<span className="text-sm text-stone-600">Use external LLM endpoint</span>
					<OnOffToggleButton
						isEnabled={settings.llmMode === "external"}
						toggleHandler={() => {
							const nextMode =
								settings.llmMode === "external" ? "local" : "external";
							if (nextMode === "external" && !settings.extLlmBaseUrl) {
								openSnackbar(
									false,
									"Set an external endpoint URL first, then enable this"
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
						<p className="font-semibold mb-1">HuggingFace access token (optional)</p>
						<p className="text-stone-500 mb-2">
							Authenticated downloads are faster and never hit HuggingFace's
							anonymous rate limits. Create a free read token at
							huggingface.co/settings/tokens.
						</p>
						<div className="flex gap-2">
							<input
								type="password"
								value={settings.hfToken ?? ""}
								onChange={updateField("hfToken")}
								className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 flex-1 p-2"
								placeholder="hf_..."
							/>
							<PinkButton
								onClick={() =>
									saveAiSettingsApi(settings)
										.then(() => openSnackbar(true, "Token saved"))
										.catch((e) => openSnackbar(false, e))
								}
							>
								Save Token
							</PinkButton>
						</div>
					</div>
				)}
			</div>

			<hr className="my-6 border-stone-200" />

			<div className="flex flex-col gap-3">
				<div className="flex justify-between items-center">
					<h3 className="font-semibold">Speech-to-text model (transcribes you)</h3>
					<span
						className={`text-xs font-medium uppercase rounded-full px-2 py-1 ${
							sttStatus.state === "ready"
								? "bg-green-100 text-green-700"
								: sttStatus.state === "error"
									? "bg-red-100 text-red-700"
									: "bg-stone-100 text-stone-600"
						}`}
					>
						{STATUS_LABELS[sttStatus.state] ?? sttStatus.state}
					</span>
				</div>
				{sttStatus.error && (
					<p className="text-sm text-red-600">{sttStatus.error}</p>
				)}
				{models.stt.map(renderModelRow)}
			</div>

			<hr className="my-6 border-stone-200" />

			<div className="flex flex-col gap-4">
				<h3 className="font-semibold">External endpoints (optional)</h3>
				<p className="text-sm text-stone-500 -mt-2">
					Offload AI to any OpenAI-compatible server (Ollama, llama.cpp server, LM
					Studio, ...). Base URL example: <code>http://localhost:11434</code>
				</p>
				<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
					<div>
						<label className="block mb-1 text-sm font-medium text-stone-900">
							LLM base URL
						</label>
						<input
							type="text"
							value={settings.extLlmBaseUrl ?? ""}
							onChange={updateField("extLlmBaseUrl")}
							className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 block w-full p-2"
							placeholder="http://localhost:11434"
						/>
					</div>
					<div>
						<label className="block mb-1 text-sm font-medium text-stone-900">
							LLM model
						</label>
						<input
							type="text"
							value={settings.extLlmModel ?? ""}
							onChange={updateField("extLlmModel")}
							className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 block w-full p-2"
							placeholder="llama3.1:8b"
						/>
					</div>
					<div>
						<label className="block mb-1 text-sm font-medium text-stone-900">
							LLM API key (if needed)
						</label>
						<input
							type="password"
							value={settings.extLlmApiKey ?? ""}
							onChange={updateField("extLlmApiKey")}
							className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 block w-full p-2"
						/>
					</div>
					<div>
						<label className="block mb-1 text-sm font-medium text-stone-900">
							STT base URL
						</label>
						<input
							type="text"
							value={settings.extSttBaseUrl ?? ""}
							onChange={updateField("extSttBaseUrl")}
							className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 block w-full p-2"
							placeholder="http://localhost:8080"
						/>
					</div>
					<div>
						<label className="block mb-1 text-sm font-medium text-stone-900">
							STT model
						</label>
						<input
							type="text"
							value={settings.extSttModel ?? ""}
							onChange={updateField("extSttModel")}
							className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 block w-full p-2"
							placeholder="whisper-1"
						/>
					</div>
					<div>
						<label className="block mb-1 text-sm font-medium text-stone-900">
							STT API key (if needed)
						</label>
						<input
							type="password"
							value={settings.extSttApiKey ?? ""}
							onChange={updateField("extSttApiKey")}
							className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 block w-full p-2"
						/>
					</div>
				</div>
				<div className="flex gap-3">
					<PinkButton
						onClick={() =>
							saveAiSettingsApi(settings)
								.then(() => {
									refresh();
									openSnackbar(true, "Settings saved");
								})
								.catch((e) => openSnackbar(false, e))
						}
					>
						Save Endpoints
					</PinkButton>
					<BorderedButton
						onClick={() =>
							testLlmEndpointApi()
								.then((msg) => openSnackbar(true, msg))
								.catch((e) => openSnackbar(false, e))
						}
					>
						Test LLM
					</BorderedButton>
					<BorderedButton
						onClick={() =>
							testSttEndpointApi()
								.then((msg) => openSnackbar(true, msg))
								.catch((e) => openSnackbar(false, e))
						}
					>
						Test STT
					</BorderedButton>
				</div>
			</div>
		</Card>
	);
}

export default AiModelsCard;
