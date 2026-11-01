import { useRef, useState, useId } from "react";

import { Card } from "./ProfileCards";
import OnOffToggleButton from "@ds/OnOffToggleButton";
import { setQuickCaptureEnabledApi } from "@helpers/api/settings";
import { useApiAction } from "@src/hooks/useApiAction";

interface QuickCaptureCardProps {
	enabled: boolean;
}

/**
 * The global quick capture shortcut: Cmd+Shift+Space (Ctrl+Shift+Space
 * on Windows and Linux) opens a new idea from anywhere, even while the
 * window hides in the tray. Disabling releases the combination for other
 * apps; enabling can fail when another app already owns it, which is
 * surfaced instead of pretending the shortcut exists.
 */
export function QuickCaptureCard({ enabled }: QuickCaptureCardProps) {
	const { run } = useApiAction();
	const labelId = useId();
	const [isEnabled, setIsEnabled] = useState(enabled);
	// Overlapping toggles: the switch stays optimistic while a save is in
	// flight, then settles on the newest save the backend accepted (the
	// same guard as AppPresenceCard).
	const seqRef = useRef(0);
	const pendingRef = useRef(0);
	const confirmedRef = useRef({ seq: 0, enabled });

	const save = (next: boolean) => {
		const seq = ++seqRef.current;
		pendingRef.current += 1;
		setIsEnabled(next);
		void run(setQuickCaptureEnabledApi(next), { success: "Saved" })
			.then((outcome) => {
				// only a resolved action confirms the switch(es)
				if (outcome.ok && seq > confirmedRef.current.seq) {
					confirmedRef.current = { seq, enabled: next };
				}
			})
			.finally(() => {
				pendingRef.current -= 1;
				if (pendingRef.current === 0) {
					setIsEnabled(confirmedRef.current.enabled);
				}
			});
	};

	return (
		<Card
			columns={3}
			title="Quick Capture"
			subtitle="Start an idea without finding the window first."
		>
			<div className="flex justify-between items-center gap-4">
				<div>
					<p className="text-sm font-medium text-stone-900" id={labelId}>
						Global shortcut
					</p>
					<p className="text-xs text-stone-500">
						<kbd className="rounded border border-stone-300 bg-stone-100 px-1 font-mono text-[11px]">
							&#8984;&#8679;Space
						</kbd>{" "}
						(
						<kbd className="rounded border border-stone-300 bg-stone-100 px-1 font-mono text-[11px]">
							Ctrl+Shift+Space
						</kbd>{" "}
						on Windows and Linux) opens a new idea from anywhere, even while Brainstory
						is hidden in the tray. If another app already uses the shortcut, enabling
						fails and it stays off.
					</p>
				</div>
				<OnOffToggleButton aria-labelledby={labelId} checked={isEnabled} onToggle={save} />
			</div>
		</Card>
	);
}

export default QuickCaptureCard;
