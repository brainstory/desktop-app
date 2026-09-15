import Button, { type ButtonProps } from "./Button";

export default function TransparentButton({
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
			classes={`bg-transparent hover:font-bold hover:bg-stone-400 hover:bg-opacity-20 text-black ${classes}`}
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
