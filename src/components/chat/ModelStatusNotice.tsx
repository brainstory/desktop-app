import { useStore } from "@nanostores/react";
import type { LlmAvailability } from "@components/global/aiStatusStore";
import { $downloads } from "@components/global/downloadStore";

import { aiSettingsHref } from "@helpers/helpers";
import { describeError } from "@helpers/describeError";

/** One-line chat notice for a language model that can't answer (yet). */
export function ModelStatusNotice({
	availability,
	error
}: {
	availability: LlmAvailability;
	/** the backend's load error, for the "error" state */
	error?: string;
}) {
	// live download progress: "no model" is not the whole story while a
	// model is on its way in
	const downloads = Object.values(useStore($downloads).active);
	const boxStyle = "mx-4 mt-2 text-sm rounded-lg px-4 py-2";
	if (availability === "loading") {
		return (
			<p role="status" className={`${boxStyle} text-stone-500 bg-stone-100`}>
				Loading the AI model&hellip; you can type already, sending unlocks when it is ready.
			</p>
		);
	}
	if (availability === "missing") {
		if (downloads.length > 0) {
			const first = downloads[0]!;
			const what = downloads.length === 1 ? first.label : `${downloads.length} models`;
			const pct = first.pct < 0 ? null : Math.min(100, Math.floor(first.pct));
			return (
				<p role="status" className={`${boxStyle} text-stone-600 bg-stone-100`}>
					Downloading {what}
					{pct === null ? "…" : ` · ${pct}%`} — chatting unlocks as soon as it&rsquo;s
					ready.{" "}
					<a className="underline font-semibold" href={aiSettingsHref()}>
						AI Models
					</a>
				</p>
			);
		}
		return (
			<p role="status" className={`${boxStyle} text-amber-900 bg-amber-50`}>
				No AI model is downloaded yet.{" "}
				<a className="underline font-semibold" href={aiSettingsHref()}>
					Download a model in Settings
				</a>{" "}
				to start chatting.
			</p>
		);
	}
	if (availability === "error") {
		const described = error ? describeError(error) : null;
		const friendly =
			described && described.kind !== "unknown"
				? `${described.message} ${described.action ?? ""}`.trim()
				: null;
		return (
			<p role="status" className={`${boxStyle} text-amber-900 bg-amber-50`}>
				The AI model failed to load{friendly ? `: ${friendly}` : error ? `: ${error}` : "."}{" "}
				<a className="underline font-semibold" href={aiSettingsHref()}>
					Check Settings &gt; AI Models
				</a>
			</p>
		);
	}
	// unknown (not reported yet) or ready: nothing to say
	return null;
}
