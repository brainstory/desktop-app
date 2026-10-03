import { useMemo, useState, type ChangeEvent } from "react";

import Button from "@ds/Button";
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

/** The zone the OS/webview reports for this machine. */
export function detectedTimezone(): string {
	return Intl.DateTimeFormat().resolvedOptions().timeZone;
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
	/** Resolves true once persisted, false if the save failed. */
	saveSettings: (name: string, timezone: string) => Promise<boolean>;
}

export function GeneralCard({ userName, timezone, saveSettings }: GeneralCardProps) {
	const [editedName, setEditedName] = useState(userName ?? "");
	const [selectedTimezone, setSelectedTimezone] = useState(timezone ?? "");
	const [errorMessage, setErrorMessage] = useState<string | undefined>(undefined);
	const [isSaving, setIsSaving] = useState(false);
	const timezones = useMemo(() => timezoneOptions(timezone), [timezone]);
	// the last CONFIRMED-saved snapshot: dirty state is derived from it,
	// so a failed save stays retryable and edits made while a save was
	// pending stay dirty without tracking the latest values in a ref
	const [saved, setSaved] = useState({ name: userName ?? "", timezone: timezone ?? "" });
	const hasChanged = editedName !== saved.name || selectedTimezone !== saved.timezone;

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
	};

	const handleTimezoneSelect = (e: ChangeEvent<HTMLSelectElement>): void => {
		setSelectedTimezone(e.target.value);
	};

	const handleSaveClick = () => {
		if (isSaving) return;
		if (editedName.trim().length > 0 && editedName.length <= 70) {
			// "Detect automatically" stores the zone detected right now, the
			// same thing userStore does on first launch; an empty string
			// would be dropped by saveUserSettingsApi and never persist
			const submitted = { name: editedName, timezone: selectedTimezone };
			setIsSaving(true);
			void saveSettings(submitted.name, submitted.timezone || detectedTimezone())
				.then((savedOutcome) => {
					if (savedOutcome) {
						// only the submitted snapshot becomes clean: edits
						// made while the save was pending stay dirty
						setSaved(submitted);
					}
				})
				.finally(() => setIsSaving(false));
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

export default {
	PhotoNameCard,
	GeneralCard
};
