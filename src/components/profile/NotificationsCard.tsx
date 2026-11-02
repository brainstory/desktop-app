import { useId, useState } from "react";
import type { NotificationSetting } from "@helpers/api/settings";
import { sendTestNotificationApi } from "@helpers/api/settings";
import { normalizeApiError } from "@helpers/helpers";

import { Card } from "./ProfileCards";
import { useDirtySave } from "@src/hooks/useDirtySave";

import OnOffToggleButton from "@ds/OnOffToggleButton";
import Button from "@ds/Button";
import { useSnackbar } from "@ds/Snackbar";

/** Same reminder list: same entries, same order, same field values. */
const notificationsEqual = (a: NotificationSetting[], b: NotificationSetting[]): boolean =>
	a.length === b.length &&
	a.every((entry, i) => {
		const other = b[i];
		return (
			other !== undefined &&
			entry.title === other.title &&
			entry.enabled === other.enabled &&
			(entry.value ?? "") === (other.value ?? "")
		);
	});

interface NotificationsCardProps {
	notificationsData?: NotificationSetting[];
	/** The stored user timezone: the reminder hour is wall-clock time
	 * in this zone (the OS zone when none is stored). */
	timeZone?: string | null;
	/** Resolves true once persisted, false if the save failed. */
	saveSettings: (fields: NotificationSetting[]) => Promise<boolean>;
}

export function NotificationsCard({
	notificationsData = [],
	timeZone,
	saveSettings
}: NotificationsCardProps) {
	const { openSnackbar } = useSnackbar();
	const [notificationFields, setNotificationFields] =
		useState<NotificationSetting[]>(notificationsData);
	const titleIdBase = useId();
	// dirty is derived from the last confirmed-saved list: edits made
	// while a save was pending stay dirty, a failed save stays retryable
	const { isSaving, isDirty, save } = useDirtySave(notificationsData, notificationsEqual);
	// fresh data from the parent replaces local state only when nothing
	// was edited (the documented "adjust state when props change"
	// render-time pattern): a reload triggered by saving ANOTHER card
	// must not wipe unsaved edits here
	const [prevData, setPrevData] = useState(notificationsData);
	if (prevData !== notificationsData) {
		setPrevData(notificationsData);
		if (!isDirty(notificationFields)) {
			setNotificationFields(notificationsData);
		}
	}
	const hasChanged = isDirty(notificationFields);

	const handleSaveClick = () => {
		void save(notificationFields, saveSettings);
	};

	const handleTestClick = () => {
		void sendTestNotificationApi()
			.then(() => {
				// the send succeeded as far as the OS reports: if no banner
				// appeared, the system suppressed it (notifications off for
				// this app - Terminal in dev builds - or a Focus mode)
				openSnackbar(
					true,
					"Test notification sent. Nothing appeared? Allow notifications for Brainstory (dev builds deliver through Terminal) in System Settings → Notifications."
				);
			})
			.catch((e) => {
				openSnackbar(false, normalizeApiError(e));
			});
	};

	const renderNotificationFields = (fields: NotificationSetting[]) => {
		return fields.map((field: NotificationSetting, i: number) => {
			const handleEnabledToggle = () => {
				// map to new objects: mutating the spread copy's elements
				// still edited the existing state in place
				setNotificationFields((prev) =>
					prev.map((entry, idx) =>
						idx === i ? { ...entry, enabled: !entry.enabled } : entry
					)
				);
			};
			const setHour = (inputDate: Date): void => {
				const hourDigitPadded = String(inputDate.getHours()).padStart(2, "0");
				setNotificationFields((prev) =>
					prev.map((entry, idx) =>
						idx === i ? { ...entry, value: `${hourDigitPadded}:00:00` } : entry
					)
				);
			};

			if (field.valueType === "time") {
				const hourDigit = Number((field.value ?? "00").split(":")[0]);
				const titleId = `${titleIdBase}-title-${i}`;

				return (
					<div className="flex justify-between" key={i}>
						<div className="flex flex-col">
							<h3 className="font-medium mb-1" id={titleId}>
								{field.title}
							</h3>
							<p className="text-sm leading-snug italic">{field.description}</p>
						</div>
						<div className="flex flex-wrap gap-1 justify-end">
							<OnOffToggleButton
								aria-labelledby={titleId}
								checked={field.enabled}
								onToggle={handleEnabledToggle}
							/>
							<div className="flex items-center flex-nowrap m-1 gap-1">
								{/* a native time input: the custom picker swallowed
								    paste, IME and on-screen keyboards (its own
								    onKeyDown handled every keystroke); step=3600
								    keeps whole hours */}
								<input
									type="time"
									step={3600}
									value={`${String(hourDigit).padStart(2, "0")}:00`}
									disabled={!field.enabled}
									aria-label={`${field.title} hour`}
									onChange={(e) => {
										// "" = cleared: keep the previous hour
										const picked = e.target.value;
										if (!picked) return;
										setHour(
											new Date(
												new Date().setHours(
													Number(picked.split(":")[0] || 0),
													0,
													0,
													0
												)
											)
										);
									}}
									className="border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-accent-500 focus:border-accent-500 p-1.5 bg-white"
								/>
								{/* the hour the picker edits is wall-clock time in
								the user's chosen zone, not the machine's */}
								{timeZone && (
									<p className="text-xs text-stone-600">
										{timeZone.replaceAll("_", " ")} time
									</p>
								)}
							</div>
						</div>
					</div>
				);
			}
		});
	};

	if (notificationFields.length === 0) return null;

	return (
		<Card columns={3} title="Notifications">
			<div className="flex flex-col gap-4">
				{renderNotificationFields(notificationFields)}
			</div>

			<div className="flex flex-wrap gap-2 justify-center mt-8">
				<Button variant="pink" disabled={!hasChanged || isSaving} onClick={handleSaveClick}>
					Save
				</Button>
				<Button variant="bordered" onClick={handleTestClick}>
					Send test notification
				</Button>
			</div>
		</Card>
	);
}
