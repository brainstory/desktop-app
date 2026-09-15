import { useState } from "react";
import type { ChangeEvent } from "react";

interface ToggleButtonProps {
	id?: string | null;
	checkedState?: string;
	uncheckedState?: string;
	defaultChecked?: boolean;
	onToggle: (newCheckedState: boolean) => void;
}

export default function ToggleButton({
	id = null,
	checkedState,
	uncheckedState,
	defaultChecked = false,
	onToggle
}: ToggleButtonProps) {
	const [isChecked, setIsChecked] = useState(defaultChecked);

	const handleCheckboxChange = (event: ChangeEvent<HTMLInputElement>) => {
		const newCheckedState = event.target.checked;
		setIsChecked(newCheckedState);

		// Call callback with updated state
		onToggle(newCheckedState);
	};

	return (
		<label id={id ?? undefined} className="toggle">
			<input type="checkbox" checked={isChecked} onChange={handleCheckboxChange} />
			<span className="slider">{isChecked ? checkedState : uncheckedState}</span>
		</label>
	);
}
