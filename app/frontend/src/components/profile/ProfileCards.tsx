import { useMemo, useState, type ChangeEvent } from "react";

import PinkButton from "@ds/PinkButton";
import { formatISO8601ToHumanReadable } from "@helpers/helpers";

interface CardProps {
	children?: React.ReactNode;
	title?: string;
	subtitle?: string;
	columns?: 1 | 2 | 3;
	classes?: string;
}

export function Card({ children, title, subtitle, columns = 1, classes = "" }: CardProps) {
	let colspan = "lg:col-span-1";
	if (columns === 2) {
		colspan = "lg:col-span-2";
	} else if (columns === 3) {
		colspan = "lg:col-span-3";
	}

	return (
		<div
			className={`w-full bg-white p-4 sm:p-8 sm:border border-stone-200 sm:rounded-lg sm:shadow ${colspan} col-span-full ${classes}`}
		>
			{title && (
				<div className="mb-6">
					<h2 className="font-semibold text-xl mb-1">{title}</h2>
					{subtitle && <h2 className="italic text-sm">{subtitle}</h2>}
				</div>
			)}
			{children}
		</div>
	);
}

interface PhotoNameCardProps {
	userName?: string | null;
	createdAt?: string | null;
	openSnackbar?: (isSuccess: boolean, message: string) => void;
}

export function PhotoNameCard({ userName, createdAt }: PhotoNameCardProps) {
	return (
		<Card columns={1}>
			<div className="bg-stone-200 flex items-center justify-center rounded-md text-4xl text-pink-600 relative w-32 h-32">
				{userName ? userName.trim().charAt(0).toUpperCase() : "?"}
			</div>
			<p className="text-2xl font-semibold mt-4 mb-1 truncate">{userName}</p>
			<p className="text-stone-600 mb-1 truncate">Local account</p>
			<p className="text-xs text-stone-600 mt-4 mb-1 truncate">
				Joined on{" "}
				{formatISO8601ToHumanReadable(createdAt ?? "", {
					month: "short",
					day: "numeric",
					year: "numeric"
				})}
			</p>
		</Card>
	);
}

let runtimeTimezones: string[] | null = null;

/**
 * The zones the timezone select offers. `Intl.supportedValuesOf` lists
 * canonical IANA zones only - Chromium (and Node) omit "UTC", "Etc/GMT"
 * and legacy aliases like "US/Pacific" - so a stored value such as "UTC"
 * would match no option and show as "Detect automatically". Always offer
 * "UTC" and the stored value.
 */
export function timezoneOptions(stored?: string | null): string[] {
	runtimeTimezones ??= Intl.supportedValuesOf("timeZone");
	if (runtimeTimezones.includes("UTC") && (!stored || runtimeTimezones.includes(stored))) {
		return runtimeTimezones;
	}
	const zones = new Set(runtimeTimezones);
	zones.add("UTC");
	if (stored) zones.add(stored);
	return [...zones].sort();
}

interface GeneralCardProps {
	userName?: string | null;
	timezone?: string;
	saveSettings: (name: string, timezone: string) => void;
}

export function GeneralCard({ userName, timezone, saveSettings }: GeneralCardProps) {
	const [editedName, setEditedName] = useState(userName ?? "");
	const [selectedTimezone, setSelectedTimezone] = useState(timezone ?? "Etc/GMT");
	const [errorMessage, setErrorMessage] = useState<string | undefined>(undefined);
	const [hasChanged, setHasChanged] = useState(false);
	const timezones = useMemo(() => timezoneOptions(timezone), [timezone]);

	const handleNameChange = (e: ChangeEvent<HTMLInputElement>): void => {
		const input = e.target.value;
		setEditedName(input);
		if (input.length <= 0) {
			setErrorMessage("Name cannot be empty");
		} else if (input.length > 70) {
			setErrorMessage("Name cannot be over 70 characters");
		} else {
			setErrorMessage(undefined);
		}
		const updateHasChanged = input !== userName || selectedTimezone !== timezone;
		setHasChanged(updateHasChanged);
	};

	const handleTimezoneSelect = (e: ChangeEvent<HTMLSelectElement>): void => {
		const input = e.target.value;
		setSelectedTimezone(input);
		// compare against the *timezone*, not the user name
		const updateHasChanged = editedName !== userName || input !== timezone;
		setHasChanged(updateHasChanged);
	};

	const handleSaveClick = () => {
		if (editedName.trim().length > 0 && editedName.length <= 70) {
			saveSettings(editedName, selectedTimezone);
			setHasChanged(false);
		}
	};

	return (
		<Card columns={2} title="General Information">
			<div className="mb-4">
				<label
					htmlFor="user-name-input"
					className="block mb-2 text-sm font-medium text-stone-900"
				>
					Your name
				</label>
				<input
					id="user-name-input"
					value={editedName}
					type="text"
					className={`border text-stone-900 text-sm rounded-lg focus:ring-accent-500 focus:border-accent-500 block w-full ${
						errorMessage ? "border-pink-600" : "border-stone-300"
					}`}
					onChange={handleNameChange}
				/>
				{errorMessage && <p className="mt-1 text-pink-600 text-sm">{errorMessage}</p>}
			</div>
			<div className="[&_:focus-visible]:ring-0">
				<label
					htmlFor="timezone-select"
					className="block mb-2 text-sm font-medium text-stone-900"
				>
					Your timezone
				</label>
				<select
					id="timezone-select"
					aria-label="Your timezone"
					value={selectedTimezone}
					onChange={handleTimezoneSelect}
					className="w-full border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-accent-500 focus:border-accent-500 p-2 bg-white"
				>
					<option value="">Detect automatically</option>
					{timezones.map((tz) => (
						<option key={tz} value={tz}>
							{tz.replaceAll("_", " ")}
						</option>
					))}
				</select>
			</div>
			<PinkButton disabled={!hasChanged} onClick={handleSaveClick} classes="mt-6 mx-auto">
				Save
			</PinkButton>
		</Card>
	);
}

export default {
	PhotoNameCard,
	GeneralCard
};
