/** The AI error banner: amber warning + settings link + dismiss. */

import { describeError } from "@helpers/describeError";

export function ChatErrorBanner({
	aiError,
	onDismiss
}: {
	aiError: string;
	onDismiss: () => void;
}) {
	const described = describeError(aiError);
	return (
		<div
			role="alert"
			className="flex items-center justify-between gap-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 m-4 text-sm"
		>
			<span>
				<b>Hmm, the AI couldn&rsquo;t respond:</b> {described.message}
				{described.action && <> {described.action}</>}
			</span>
			<span className="flex gap-2 shrink-0">
				<a
					className="underline font-semibold whitespace-nowrap"
					href="/profile?tab=aiModels"
				>
					Open AI settings
				</a>
				<button className="underline text-stone-500 whitespace-nowrap" onClick={onDismiss}>
					Dismiss
				</button>
			</span>
		</div>
	);
}
