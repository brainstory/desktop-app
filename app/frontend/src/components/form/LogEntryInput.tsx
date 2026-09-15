

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
	return (
		<div className="flex justify-between items-center text-sm md:text-md leading-snug">
			<p className="pr-2">{text}</p>
			<OnOffToggleButton
				id={id}
				checkedState={checkedState}
				uncheckedState={uncheckedState}
				defaultChecked={value}
				onToggle={onChange ?? (() => {})}
				{...props}
			/>
		</div>
	);
}
