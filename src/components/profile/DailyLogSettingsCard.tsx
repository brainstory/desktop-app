import { useState } from "react";

import { Card } from "./ProfileCards";
import LogEntry from "@components/form/LogEntryInput";
import type { LogSettingsQuestion } from "@helpers/api/settings";
import Button from "@ds/Button";

const getEnabledLogQidsFromData = (apiData: LogSettingsQuestion[]): number[] => {
	return apiData.reduce((acc: number[], currField: LogSettingsQuestion) => {
		if (currField.enabled) {
			acc.push(currField.id);
			return acc;
		}
		return acc;
	}, []);
};

interface DailyLogSettingsCardProps {
	logFieldsData?: LogSettingsQuestion[];
	/** Resolves true once persisted, false if the save failed. */
	saveSettings: (ids: number[]) => Promise<boolean>;
}

export function DailyLogSettingsCard({
	logFieldsData = [],
	saveSettings
}: DailyLogSettingsCardProps) {
	const initialQids = getEnabledLogQidsFromData(logFieldsData);
	const [enabledLogQids, setEnabledLogQids] = useState(initialQids);
	// the last CONFIRMED-saved snapshot: dirty state is derived from it,
	// so a failed save stays retryable and toggles made while a save was
	// pending stay dirty without tracking the latest values in a ref
	const [savedQids, setSavedQids] = useState(initialQids);
	const [errorMessage, setErrorMessage] = useState<string | undefined>(undefined);
	const [isSaving, setIsSaving] = useState(false);
	const hasChanged =
		enabledLogQids.length !== savedQids.length ||
		!enabledLogQids.every((id) => savedQids.includes(id));

	const handleToggle = (toggledId: number): void => {
		let updatedEnabledLogQids = [...enabledLogQids];
		if (enabledLogQids.includes(toggledId)) {
			updatedEnabledLogQids = updatedEnabledLogQids.filter((id) => id !== toggledId);
		} else {
			updatedEnabledLogQids.push(toggledId);
		}
		setEnabledLogQids(updatedEnabledLogQids);
	};

	const handleSaveClick = () => {
		if (isSaving) return;
		if (enabledLogQids.length === 0) {
			setErrorMessage("Must have at least 1 question enabled");
			return;
		}
		const submittedQids = enabledLogQids;
		setIsSaving(true);
		setErrorMessage(undefined);
		void saveSettings(submittedQids)
			.then((saved) => {
				if (saved) {
					// only the submitted ids become clean: toggles made
					// while the save was pending stay dirty
					setSavedQids(submittedQids);
				}
			})
			.finally(() => setIsSaving(false));
	};

	return (
		<Card
			columns={3}
			title="Daily Log Questions"
			subtitle="Included at the start of every daily intent"
			classes=""
		>
			<div className="flex flex-col gap-4">
				{renderlogFields(logFieldsData, enabledLogQids, handleToggle)}
			</div>
			{errorMessage && <p className="mt-1 text-pink-600 text-sm">{errorMessage}</p>}
			<Button
				variant="pink"
				disabled={!hasChanged || isSaving}
				onClick={handleSaveClick}
				classes="mt-6 mx-auto"
			>
				Save
			</Button>
		</Card>
	);
}

interface RenderField {
	id: number;
	label: string;
	text: string;
	value: boolean;
}

const renderlogFields = (
	logFieldsApiData: LogSettingsQuestion[],
	enabledLogQids: number[],
	handleToggle: (id: number) => void
) => {
	// First, format api array data to an object with key as label
	const fieldLabels: string[] = [];
	const labelToFields: Record<string, RenderField[]> = {};
	logFieldsApiData.forEach((field) => {
		const label = field.label;
		const fieldData = {
			id: field.id,
			label: field.label,
			text: field.questionText,
			value: enabledLogQids.includes(field.id)
		};
		if (fieldLabels.includes(label)) {
			labelToFields[label]!.push(fieldData);
		} else {
			labelToFields[label] = [fieldData];
			fieldLabels.push(label);
		}
	});

	// Iterate over every category of fields, then render each field
	return fieldLabels.map((label) => {
		const fieldsInLabel = labelToFields[label];
		return (
			<div key={label}>
				<h3 className="font-medium underline decoration-pink-500 my-1">{label}</h3>
				<div className="flex flex-col gap-1">
					{fieldsInLabel!.map((field, i) => {
						const lineBreak =
							fieldsInLabel?.length == i + 1 ? null : (
								<div key={"divider-" + field.id} className="h-[1px] bg-slate-200" />
							);
						return [
							<LogEntry
								key={field.id}
								onChange={() => handleToggle(field.id)}
								checkedState="On"
								uncheckedState="Off"
								{...field}
							/>,
							lineBreak
						];
					})}
				</div>
			</div>
		);
	});
};
