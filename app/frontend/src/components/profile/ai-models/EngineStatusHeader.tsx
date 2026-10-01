import { cn } from "@helpers/cn";
import type { EngineStatus } from "@helpers/api/models";

const STATUS_LABELS = {
	ready: "Ready",
	loading: "Loading...",
	missing: "Not downloaded",
	error: "Error",
	external: "Using external endpoint"
};

/** Section heading with the engine's status badge and its error, if any. */
export function EngineStatusHeader({
	title,
	status,
	headingId
}: {
	title: string;
	status: EngineStatus;
	/** id for the heading, so the section can be labelled by it */
	headingId?: string;
}) {
	return (
		<>
			<div className="flex justify-between items-center">
				<h3 id={headingId} className="font-semibold">
					{title}
				</h3>
				<span
					className={cn(
						"text-xs font-medium uppercase rounded-full px-2 py-1",
						status.state === "ready"
							? "bg-green-100 text-green-700"
							: status.state === "error"
								? "bg-red-100 text-red-700"
								: "bg-stone-100 text-stone-600"
					)}
				>
					{STATUS_LABELS[status.state] ?? status.state}
				</span>
			</div>
			{status.error && <p className="text-sm text-red-600">{status.error}</p>}
		</>
	);
}
