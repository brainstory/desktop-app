import { useId, useState } from "react";
import type { NotificationSetting } from "@helpers/api/settings";

import { Card } from "./ProfileCards";
import TimePickerInput from "@components/global/time-picker/TimePickerInput";

import OnOffToggleButton from "@ds/OnOffToggleButton";
import PinkButton from "@ds/PinkButton";

interface NotificationsCardProps {
	notificationsData?: NotificationSetting[];
	/** Resolves true once persisted, false if the save failed. */
	saveSettings: (fields: NotificationSetting[]) => Promise<boolean>;
}

export function NotificationsCard({
	notificationsData = [],
	saveSettings
}: NotificationsCardProps) {
	const [notificationFields, setNotificationFields] =
		useState<NotificationSetting[]>(notificationsData);
	const [hasChanged, setHasChanged] = useState(false);
	const titleIdBase = useId();
	// reset local edits whenever the parent passes fresh data (the
	// documented "adjust state when props change" render-time pattern)
	const [prevData, setPrevData] = useState(notificationsData);
	if (prevData !== notificationsData) {
		setPrevData(notificationsData);
		setNotificationFields(notificationsData);
	}

	const handleSaveClick = () => {
		setHasChanged(false);
		void saveSettings(notificationFields).then((saved) => {
			// a failed save leaves the edits pending, so Save can be retried
			if (!saved) setHasChanged(true);
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
				setHasChanged(true);
			};
			const setHour = (inputDate: Date): void => {
				const hourDigitPadded = String(inputDate.getHours()).padStart(2, "0");
				setNotificationFields((prev) =>
					prev.map((entry, idx) =>
						idx === i ? { ...entry, value: `${hourDigitPadded}:00:00` } : entry
					)
				);
				setHasChanged(true);
			};

			if (field.valueType === "time") {
				const hourDigit = Number((field.value ?? "00").split(":")[0]);
				const hourAsDate = new Date(new Date().setHours(hourDigit, 0, 0, 0));
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
								<TimePickerInput
									picker="hours"
									date={hourAsDate}
									setDate={setHour}
									disabled={!field.enabled}
									aria-label={`${field.title} reminder hour`}
								/>
								<p className="font-mono tabular-nums text-sm" aria-hidden="true">
									:00
								</p>
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

			<PinkButton disabled={!hasChanged} onClick={handleSaveClick} classes="mt-8 mx-auto">
				Save
			</PinkButton>
		</Card>
	);
}
