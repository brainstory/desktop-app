import { useEffect, useState } from "react";
import { useStore } from "@nanostores/react";
import { listModelsApi, getAiSettingsApi } from "@helpers/api/models";
import PinkButton from "@ds/PinkButton";
import { $aiStatus } from "@components/global/aiStatusStore";

/**
 * Shown until the app has an AI brain: either a downloaded local model or a
 * configured external endpoint. First-run users otherwise only discover this
 * when a session fails.
 */
export default function AiSetupNeeded() {
	const [needsSetup, setNeedsSetup] = useState(false);
	const [checked, setChecked] = useState(false);
	const [smallestGb, setSmallestGb] = useState<string | null>(null);
	// live engine status from the shared store (fed once per app)
	useStore($aiStatus);

	useEffect(() => {
		Promise.all([listModelsApi(), getAiSettingsApi()])
			.then(([models, settings]) => {
				const hasLocalModel = models.llm.some((m) => m.downloaded);
				const hasExternal = !!settings.extLlmBaseUrl;
				setNeedsSetup(!hasLocalModel && !hasExternal);
				// copy derives from the catalog instead of hardcoding a size
				const smallest = models.llm.reduce((min, m) =>
					m.sizeBytes < min.sizeBytes ? m : min
				);
				setSmallestGb((smallest.sizeBytes / 1024 ** 3).toFixed(1));
				setChecked(true);
			})
			.catch(() => setChecked(true));
	}, []);

	if (!checked || !needsSetup) {
		return null;
	}

	return (
		<div className="mb-6 border-2 border-pink-200 bg-pink-50 rounded-lg p-6 text-center">
			<p className="font-semibold text-lg text-stone-900 mb-1">
				Welcome! One quick step before your first Brainstory
			</p>
			<p className="text-sm text-stone-600 mb-4 max-w-xl mx-auto">
				Everything runs on your machine, so Brainstory needs an AI brain to brainstorm with:
				download a local model{smallestGb ? ` (~${smallestGb} GB, once)` : ""}, or point it
				at an external AI server if you have one. You can change this any time.
			</p>
			<PinkButton href="/profile?tab=aiModels">Set up AI Models</PinkButton>
		</div>
	);
}
