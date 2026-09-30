import { useState } from "react";
import { normalizeApiError } from "@helpers/helpers";

import { Card } from "./ProfileCards";
import OnOffToggleButton from "@ds/OnOffToggleButton";
import { setAppPresenceApi } from "@helpers/api/settings";

interface AppPresenceCardProps {
	presence: { dock: boolean; tray: boolean };
	openSnackbar: (isSuccess: boolean, message: string) => void;
}

export function AppPresenceCard({ presence, openSnackbar }: AppPresenceCardProps) {
	// The switches are controlled from here; a failed save reverts the
	// optimistic flip instead of leaving the UI disagreeing with reality.
	const [dock, setDock] = useState(presence.dock);
	const [tray, setTray] = useState(presence.tray);

	const save = (nextDock: boolean, nextTray: boolean) => {
		const prev = { dock, tray };
		setDock(nextDock);
		setTray(nextTray);
		setAppPresenceApi(nextDock, nextTray)
			.then(() => openSnackbar(true, "Saved"))
			.catch((e) => {
				setDock(prev.dock);
				setTray(prev.tray);
				openSnackbar(false, normalizeApiError(e));
			});
	};

	return (
		<Card title="App Presence" subtitle="Where Brainstory shows up on your desktop.">
			<div className="flex flex-col gap-4">
				<div className="flex justify-between items-center gap-4">
					<div>
						<p className="text-sm font-medium text-stone-900">Show in Dock</p>
						<p className="text-xs text-stone-500">
							The Brainstory icon in the macOS Dock.
						</p>
					</div>
					<OnOffToggleButton checked={dock} onToggle={(enabled) => save(enabled, tray)} />
				</div>
				<div className="flex justify-between items-center gap-4">
					<div>
						<p className="text-sm font-medium text-stone-900">Show in menu bar</p>
						<p className="text-xs text-stone-500">
							Keeps Brainstory running in the background when the window is closed, so
							daily reminders still fire. If turned off, closing the window quits
							Brainstory.
						</p>
					</div>
					<OnOffToggleButton checked={tray} onToggle={(enabled) => save(dock, enabled)} />
				</div>
			</div>
		</Card>
	);
}

export default AppPresenceCard;
