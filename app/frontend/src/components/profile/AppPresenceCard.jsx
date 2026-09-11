import { Card } from "./ProfileCards";
import OnOffToggleButton from "@ds/OnOffToggleButton";
import { setAppPresenceApi } from "@helpers/api/settings";

export function AppPresenceCard({ presence, openSnackbar }) {
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
					<OnOffToggleButton
						defaultChecked={presence.dock}
						onToggle={(enabled) => {
							setAppPresenceApi(enabled, presence.tray)
								.then(() => openSnackbar(true, "Saved"))
								.catch((e) => openSnackbar(false, e));
						}}
					/>
				</div>
				<div className="flex justify-between items-center gap-4">
					<div>
						<p className="text-sm font-medium text-stone-900">Show in menu bar</p>
						<p className="text-xs text-stone-500">
							Keeps Brainstory running in the background when the window is
							closed, so daily reminders still fire. If turned off, closing
							the window quits Brainstory.
						</p>
					</div>
					<OnOffToggleButton
						defaultChecked={presence.tray}
						onToggle={(enabled) => {
							setAppPresenceApi(presence.dock, enabled)
								.then(() => openSnackbar(true, "Saved"))
								.catch((e) => openSnackbar(false, e));
						}}
					/>
				</div>
			</div>
		</Card>
	);
}

export default AppPresenceCard;
