import { useCallback, useId, useRef, useState } from "react";
import { normalizeApiError } from "@helpers/helpers";

import { Card } from "./ProfileCards";
import Button from "@ds/Button";
import OnOffToggleButton from "@ds/OnOffToggleButton";
import { setUpdatesEnabledApi } from "@helpers/api/settings";

/**
 * The "check for updates automatically" setting, owned by the settings
 * page so the switch (in the Updates card) and the warning (at the top
 * of the page) share one state. Same settle rule as App Presence:
 * optimistic while saves are in flight, then the newest value the
 * backend accepted.
 */
export function useUpdatesEnabled(openSnackbar: (isSuccess: boolean, message: string) => void) {
	const [enabled, setEnabled] = useState(true);
	const seqRef = useRef(0);
	const pendingRef = useRef(0);
	const confirmedRef = useRef({ seq: 0, enabled: true });

	/** Adopt the stored value once settings have loaded. */
	const load = useCallback((value: boolean) => {
		confirmedRef.current = { seq: seqRef.current, enabled: value };
		setEnabled(value);
	}, []);

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

	return { enabled, load, save };
}

interface UpdatesCardProps {
	enabled: boolean;
	onToggle: (enabled: boolean) => void;
}

export function UpdatesCard({ enabled, onToggle }: UpdatesCardProps) {
	const labelId = useId();
	return (
		<Card title="Updates" subtitle="How Brainstory keeps itself up to date.">
			<div className="flex justify-between items-center gap-4">
				<div>
					<p className="text-sm font-medium text-stone-900" id={labelId}>
						Check for updates automatically
					</p>
					<p className="text-xs text-stone-500">
						Looks for a new version at most every 6 hours and asks before installing it.
					</p>
				</div>
				<OnOffToggleButton
					aria-labelledby={labelId}
					checked={enabled}
					onToggle={onToggle}
				/>
			</div>
		</Card>
	);
}

interface UpdatesDisabledWarningProps {
	enabled: boolean;
	onTurnOn: () => void;
}

/**
 * Page-wide warning shown above the settings tabs while update checks
 * are off, so it can't be missed on any tab. The live region stays
 * mounted so the warning is announced when it appears.
 */
export function UpdatesDisabledWarning({ enabled, onTurnOn }: UpdatesDisabledWarningProps) {
	return (
		<div role="status" className="w-full">
			{!enabled && (
				<div className="mb-6 flex w-full items-center gap-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg px-5 py-4">
					<ion-icon
						class="w-6 h-6 shrink-0 hydrated"
						name="warning-outline"
						aria-hidden="true"
					></ion-icon>
					<p className="flex-1 text-sm">
						<span className="font-semibold">Automatic update checks are off.</span> You
						could be missing new features, bug fixes and security fixes.
					</p>
					<Button variant="bordered" classes="shrink-0" onClick={onTurnOn}>
						Turn back on
					</Button>
				</div>
			)}
		</div>
	);
}

export default UpdatesCard;
