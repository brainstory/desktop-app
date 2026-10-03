import { useState } from "react";
import Button from "./Button";

/**
 * Password input for a stored secret (token / API key). The backend never
 * sends the real value back to the UI, only whether one is stored plus a
 * masked hint. An empty field means "keep the stored value"; Remove is the
 * only way to clear it. The typed value is cleared only after onSave
 * confirmed the save; a rejected save keeps it for a retry, and clicks are
 * ignored while a save is pending.
 */
interface SecretFieldProps {
	placeholder?: string;
	stored?: boolean;
	hint?: string | null;
	saveLabel?: string;
	/** Resolves false (or rejects) when the value was not stored. */
	onSave: (value: string) => Promise<boolean> | void;
	inputClasses?: string;
	/** id for the input, so a <label htmlFor> can point at it */
	inputId?: string;
}

export default function SecretField({
	placeholder,
	stored = false,
	hint,
	saveLabel = "Save",
	onSave,
	inputClasses = "",
	inputId
}: SecretFieldProps) {
	const [value, setValue] = useState("");
	const [pending, setPending] = useState(false);
	const [error, setError] = useState<string | null>(null);

	/** Submit and wait for the confirmed outcome; `false` keeps the value. */
	const submit = async (next: string, failedMessage: string): Promise<void> => {
		if (pending) return;
		setPending(true);
		setError(null);
		let outcome: boolean | void;
		try {
			outcome = await onSave(next);
		} catch {
			outcome = false;
		}
		setPending(false);
		if (outcome === false) {
			// keep the typed secret so the user can simply retry
			setError(failedMessage);
		} else {
			// confirmed: the raw value only ever lived in this input
			setValue("");
		}
	};

	return (
		<div className="min-w-0">
			{stored && (
				<p className="text-green-700 mb-2">
					Saved ({hint}). It is stored locally and never displayed.
				</p>
			)}
			{error && (
				<p role="alert" className="text-red-600 mb-2">
					{error}
				</p>
			)}
			<div className="flex flex-wrap gap-2">
				<input
					id={inputId}
					type="password"
					value={value}
					onChange={(e) => {
						setValue(e.target.value);
						setError(null);
					}}
					className={`border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-accent-500 focus:border-accent-500 flex-1 min-w-0 p-2 ${inputClasses}`}
					placeholder={stored ? "Leave empty to keep the saved value" : placeholder}
				/>
				<Button
					variant="pink"
					disabled={pending || value === ""}
					onClick={() =>
						void submit(
							value,
							"Did not save - the typed value is kept so you can retry."
						)
					}
				>
					{saveLabel}
				</Button>
				{stored && (
					<Button
						variant="bordered"
						disabled={pending}
						onClick={() =>
							void submit(
								"",
								"Did not remove - the stored value is unchanged, try again."
							)
						}
					>
						Remove
					</Button>
				)}
			</div>
		</div>
	);
}
