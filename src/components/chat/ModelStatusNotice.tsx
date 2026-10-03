import type { LlmAvailability } from "@components/global/aiStatusStore";

const SETTINGS_HREF = "/profile?tab=aiModels";

/** One-line chat notice for a language model that can't answer (yet). */
export function ModelStatusNotice({
	availability,
	error
}: {
	availability: LlmAvailability;
	/** the backend's load error, for the "error" state */
	error?: string;
}) {
	const boxStyle = "mx-4 mt-2 text-sm rounded-lg px-4 py-2";
	if (availability === "loading") {
		return (
			<p role="status" className={`${boxStyle} text-stone-500 bg-stone-100`}>
				Loading the AI model&hellip; you can type already, sending unlocks when it is ready.
			</p>
		);
	}
	if (availability === "missing") {
		return (
			<p role="status" className={`${boxStyle} text-amber-900 bg-amber-50`}>
				No AI model is downloaded yet.{" "}
				<a className="underline font-semibold" href={SETTINGS_HREF}>
					Download a model in Settings
				</a>{" "}
				to start chatting.
			</p>
		);
	}
	if (availability === "error") {
		return (
			<p role="status" className={`${boxStyle} text-amber-900 bg-amber-50`}>
				The AI model failed to load{error ? `: ${error}` : "."}{" "}
				<a className="underline font-semibold" href={SETTINGS_HREF}>
					Check Settings &gt; AI Models
				</a>
			</p>
		);
	}
	// unknown (not reported yet) or ready: nothing to say
	return null;
}
