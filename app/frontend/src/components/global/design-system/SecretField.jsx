import { useState } from "react";
import PinkButton from "./PinkButton";
import BorderedButton from "./BorderedButton";

/**
 * Password input for a stored secret (token / API key). The backend never
 * sends the real value back to the UI, only whether one is stored plus a
 * masked hint. An empty field means "keep the stored value"; Remove is the
 * only way to clear it.
 */
export default function SecretField({
	placeholder,
	stored = false,
	hint = null,
	saveLabel = "Save",
	onSave,
	inputClasses = ""
}) {
	const [value, setValue] = useState("");

	const saveAndReset = (next) => {
		onSave(next);
		setValue("");
	};

	return (
		<div className="min-w-0">
			{stored && (
				<p className="text-green-700 mb-2">
					Saved ({hint}). It is stored locally and never displayed.
				</p>
			)}
			<div className="flex flex-wrap gap-2">
				<input
					type="password"
					value={value}
					onChange={(e) => setValue(e.target.value)}
					className={`border border-stone-300 text-stone-900 text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 flex-1 min-w-0 p-2 ${inputClasses}`}
					placeholder={
						stored ? "Leave empty to keep the saved value" : placeholder
					}
				/>
				<PinkButton
					disabled={value === ""}
					onClick={() => saveAndReset(value)}
				>
					{saveLabel}
				</PinkButton>
				{stored && (
					<BorderedButton onClick={() => saveAndReset("")}>Remove</BorderedButton>
				)}
			</div>
		</div>
	);
}
