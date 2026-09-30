import { cn } from "@helpers/cn";

/**
 * Controlled on/off switch: the parent owns the checked state (the
 * component never flips itself), so a rejected save can simply not
 * update the state and the switch reverts on the next render. A single
 * <button role="switch"> - Space and Enter activate it natively.
 */
export default function OnOffToggleButton({
	id = null,
	checkedState = "On",
	uncheckedState = "Off",
	checked,
	onToggle,
	disabled,
	"aria-labelledby": ariaLabelledby
}: {
	id?: string | number | null;
	checkedState?: string;
	uncheckedState?: string;
	checked: boolean;
	onToggle: (newCheckedState: boolean) => void;
	disabled?: boolean;
	"aria-labelledby"?: string;
}) {
	return (
		<button
			id={id === null ? undefined : String(id)}
			type="button"
			role="switch"
			aria-checked={checked}
			aria-labelledby={ariaLabelledby}
			disabled={disabled}
			onClick={() => onToggle(!checked)}
			className={cn(
				"m-1 border shadow rounded-full relative inline-flex cursor-pointer select-none items-center",
				checked
					? "transition-colors duration-150 bg-pink-200 border-pink-400"
					: "transition-colors duration-150 bg-stone-200 border-stone-300",
				disabled && "opacity-70 cursor-not-allowed"
			)}
		>
			<div
				className={cn(
					"h-[30px] w-[30px] bg-white absolute rounded-full transition-transform duration-150 ml-[4px] mr-[4px]",
					checked && "transform translate-x-[42px]"
				)}
			></div>
			<div className="flex h-[36px] rounded-md">
				<span
					aria-hidden="true"
					className={cn(
						"pl-1 text-xs font-medium flex w-[40px] items-center justify-center rounded",
						checked
							? "transition-opacity opacity-100 duration-150"
							: "transition-none opacity-0"
					)}
				>
					{checkedState}
				</span>
				<span
					aria-hidden="true"
					className={cn(
						"pr-1 text-xs font-medium flex w-[40px] items-center justify-center rounded",
						!checked
							? "transition-opacity opacity-100 duration-150"
							: "transition-none opacity-0"
					)}
				>
					{uncheckedState}
				</span>
			</div>
		</button>
	);
}
