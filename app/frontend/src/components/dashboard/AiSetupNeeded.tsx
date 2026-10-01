import { useEffect, useState } from "react";
import { useStore } from "@nanostores/react";
import { listModelsApi } from "@helpers/api/models";
import PinkButton from "@ds/PinkButton";
import { $aiStatus, initAiStatus } from "@components/global/aiStatusStore";

/** Catalog lookup state: not fetched yet, or whether any local LLM is
 * downloaded plus the smallest model's size in GB (null when unknown). */
type Catalog =
	{ loaded: false } | { loaded: true; hasLocalModel: boolean; smallestGb: string | null };

/**
 * Shown until the app has an AI brain: either a downloaded local model or a
 * configured external endpoint. First-run users otherwise only discover this
 * when a session fails.
 *
 * Derived from the shared $aiStatus store: the backend reports the LLM
 * engine as "missing" when external mode is off and the selected model is
 * not downloaded. Any other (or not yet known) state hides the card. The
 * model catalog is read only while "missing", for the size hint and to
 * keep the card to first-run users (no LLM downloaded at all).
 */
export default function AiSetupNeeded() {
	const aiStatus = useStore($aiStatus);
	const isMissing = aiStatus.llm.state === "missing";
	const [catalog, setCatalog] = useState<Catalog>({ loaded: false });

	// Make sure the store is seeded on pages without the chat AppWrapper
	// (the dashboard). Idempotent: only the first call does anything.
	useEffect(() => {
		void initAiStatus();
	}, []);

	useEffect(() => {
		if (!isMissing) return;
		let isCurrent = true;
		listModelsApi()
			.then((models) => {
				const smallestModel = models.llm.reduce<(typeof models.llm)[number] | null>(
					(min, m) => (min === null || m.sizeBytes < min.sizeBytes ? m : min),
					null
				);
				if (!isCurrent) return;
				setCatalog({
					loaded: true,
					hasLocalModel: models.llm.some((m) => m.downloaded),
					smallestGb: smallestModel
						? (smallestModel.sizeBytes / 1024 ** 3).toFixed(1)
						: null
				});
			})
			.catch(() => {
				if (isCurrent) setCatalog({ loaded: true, hasLocalModel: false, smallestGb: null });
			});
		return () => {
			isCurrent = false;
		};
	}, [isMissing]);

	if (!isMissing || !catalog.loaded || catalog.hasLocalModel) {
		return null;
	}
	const { smallestGb } = catalog;

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
