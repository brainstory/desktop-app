import Button, { type ButtonProps } from "./Button";

export default function BlackButton({
	children,
	icon = null,
	full = false,
	left = false,
	classes = "",
	disabled = false,
	sr = "",
	...props
}: ButtonProps) {
	return (
		<Button
			classes={`bg-stone-950 hover:bg-stone-800 text-white ${classes}`}
			icon={icon}
			full={full}
			left={left}
			disabled={disabled}
			sr={sr}
			{...props}
		>
			{children}
		</Button>
	);
}
