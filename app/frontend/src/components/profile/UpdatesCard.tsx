import { useId, useRef, useState } from "react";
import { normalizeApiError } from "@helpers/helpers";

import { Card } from "./ProfileCards";
import { setUpdatesEnabledApi } from "@helpers/api/settings";

interface UpdatesCardProps {
	enabled: boolean;
	openSnackbar: (isSuccess: boolean, message: string) => void;
}

export function UpdatesCard({ enabled: initialEnabled, openSnackbar }: UpdatesCardProps) {
	const checkboxId = useId();
	const helpId = useId();
	const [enabled, setEnabled] = useState(initialEnabled);
	// Same settle rule as App Presence: optimistic while saves are in
	// flight, then the newest value the backend accepted.
	const seqRef = useRef(0);
	const pendingRef = useRef(0);
	const confirmedRef = useRef({ seq: 0, enabled: initialEnabled });

	const save = (next: boolean) => {
		const seq = ++seqRef.current;
		pendingRef.current += 1;
		setEnabled(next);
		setUpdatesEnabledApi(next)
			.then(() => {
				if (seq > confirmedRef.current.seq) {
					confirmedRef.current = { seq, enabled: next };
				}
				openSnackbar(true, "Saved");
			})
			.catch((e) => openSnackbar(false, normalizeApiError(e)))
			.finally(() => {
				pendingRef.current -= 1;
				if (pendingRef.current === 0) {
					setEnabled(confirmedRef.current.enabled);
				}
			});
	};

	return (
		<Card title="Updates" subtitle="How Brainstory keeps itself up to date.">
			<div className="flex flex-col gap-4">
				<div className="flex items-start gap-3">
					<input
						id={checkboxId}
						type="checkbox"
						checked={enabled}
						onChange={(e) => save(e.target.checked)}
						aria-describedby={helpId}
						className="mt-0.5 h-4 w-4 shrink-0 cursor-pointer accent-accent-600 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-accent-300"
					/>
					<div>
						<label
							htmlFor={checkboxId}
							className="text-sm font-medium text-stone-900 cursor-pointer"
						>
							Check for updates automatically
						</label>
						<p id={helpId} className="text-xs text-stone-500">
							Looks for a new version at most every 6 hours and asks before installing
							it.
						</p>
					</div>
				</div>
				{/* the live region stays mounted so the warning is announced when it appears */}
				<div role="status">
					{!enabled && (
						<div className="flex gap-3 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 text-sm">
							<ion-icon
								class="w-5 h-5 shrink-0 hydrated"
								name="warning-outline"
								aria-hidden="true"
							></ion-icon>
							<p>
								Automatic update checks are off. You could be missing new features,
								bug fixes and security fixes.
							</p>
						</div>
					)}
				</div>
			</div>
		</Card>
	);
}

export default UpdatesCard;
