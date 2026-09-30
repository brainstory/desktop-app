import Button, { type ButtonProps } from "./Button";

export default function PinkButton({
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
			classes={`bg-accent-600 hover:bg-accent-700 text-white ${classes}`}
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
