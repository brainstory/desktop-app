import type { ReactNode } from "react";
import SecretField from "@ds/SecretField";

interface SecretRowProps {
	inputId: string;
	label: string;
	/** help text between the label and the field */
	description?: ReactNode;
	labelClassName?: string;
	stored: boolean;
	hint: string | null;
	placeholder: string;
	saveLabel?: string;
	onSave: (value: string) => void;
}

/** A labelled secret (token / API key) field. */
export function SecretRow({
	inputId,
	label,
	description,
	labelClassName = "block mb-1 text-sm font-medium text-stone-900",
	...field
}: SecretRowProps) {
	return (
		<div>
			<label htmlFor={inputId} className={labelClassName}>
				{label}
			</label>
			{description && <p className="text-stone-500 mb-2">{description}</p>}
			<SecretField inputId={inputId} {...field} />
		</div>
	);
}
