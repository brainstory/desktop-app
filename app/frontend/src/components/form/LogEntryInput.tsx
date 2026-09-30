import { useId } from "react";

import OnOffToggleButton from "@components/global/design-system/OnOffToggleButton";

interface LogEntryInputProps {
	id?: string | number;
	disabled?: boolean;
	text?: string;
	value?: boolean;
	onChange?: (newCheckedState: boolean) => void;
	checkedState?: string;
	uncheckedState?: string;
}

export default function LogEntryInput({
	id,
	text,
	value,
	onChange,
	checkedState = "Yes",
	uncheckedState = "No",
	...props
}: LogEntryInputProps) {
	const labelId = useId();
	return (
		<div className="flex justify-between items-center text-sm md:text-md leading-snug">
			<p className="pr-2" id={labelId}>
				{text}
			</p>
			<OnOffToggleButton
				id={id}
				aria-labelledby={labelId}
				checkedState={checkedState}
				uncheckedState={uncheckedState}
				checked={value ?? false}
				onToggle={onChange ?? (() => {})}
				{...props}
			/>
		</div>
	);
}
