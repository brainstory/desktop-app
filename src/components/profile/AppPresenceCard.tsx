import { useRef, useState, useId } from "react";

import { Card } from "./ProfileCards";
import OnOffToggleButton from "@ds/OnOffToggleButton";
import { setAppPresenceApi } from "@helpers/api/settings";
import { useApiAction } from "@src/hooks/useApiAction";

interface AppPresenceCardProps {
	presence: { dock: boolean; tray: boolean };
}

export function AppPresenceCard({ presence }: AppPresenceCardProps) {
	const { run } = useApiAction();
	const dockLabelId = useId();
	const trayLabelId = useId();
	// The switches are controlled from here; a failed save reverts the
	// optimistic flip instead of leaving the UI disagreeing with reality.
	const [dock, setDock] = useState(presence.dock);
	const [tray, setTray] = useState(presence.tray);
	// Quick toggles overlap: the UI stays optimistic while any save is in
	// flight, then settles on the newest save the backend accepted. A late
	// failure of an older save must not undo a newer one that landed.
	const seqRef = useRef(0);
	const pendingRef = useRef(0);
	const confirmedRef = useRef({ seq: 0, dock: presence.dock, tray: presence.tray });

	const save = (nextDock: boolean, nextTray: boolean) => {
		const seq = ++seqRef.current;
		pendingRef.current += 1;
		setDock(nextDock);
		setTray(nextTray);
		void run(setAppPresenceApi(nextDock, nextTray), { success: "Saved" })
			.then((outcome) => {
				// only a resolved action confirms the switches
				if (outcome.ok && seq > confirmedRef.current.seq) {
					confirmedRef.current = { seq, dock: nextDock, tray: nextTray };
				}
			})
			.finally(() => {
				pendingRef.current -= 1;
				if (pendingRef.current === 0) {
					setDock(confirmedRef.current.dock);
					setTray(confirmedRef.current.tray);
				}
			});
	};

	return (
		<Card
			columns={3}
			title="App Presence"
			subtitle="Where Brainstory shows up on your desktop."
		>
			<div className="flex flex-col gap-4">
				<div className="flex justify-between items-center gap-4">
					<div>
						<p className="text-sm font-medium text-stone-900" id={dockLabelId}>
							Show in Dock
						</p>
						<p className="text-xs text-stone-500">
							The Brainstory icon in the Dock / taskbar.
						</p>
					</div>
					<OnOffToggleButton
						aria-labelledby={dockLabelId}
						checked={dock}
						onToggle={(enabled) => save(enabled, tray)}
					/>
				</div>
				<div className="flex justify-between items-center gap-4">
					<div>
						<p className="text-sm font-medium text-stone-900" id={trayLabelId}>
							Show in menu bar
						</p>
						<p className="text-xs text-stone-500">
							The Brainstory icon in the system tray / menu bar. Keeps the app running
							in the background when the window is closed, so daily reminders still
							fire. If turned off, closing the window quits Brainstory.
						</p>
					</div>
					<OnOffToggleButton
						aria-labelledby={trayLabelId}
						checked={tray}
						onToggle={(enabled) => save(dock, enabled)}
					/>
				</div>
			</div>
		</Card>
	);
}

export default AppPresenceCard;
