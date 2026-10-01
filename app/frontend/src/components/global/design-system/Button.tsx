import React from "react";
import { cn } from "@helpers/cn";

/** Colour schemes (formerly PinkButton / BlackButton / BorderedButton /
 * TransparentButton). Without a variant the button is uncoloured. */
const VARIANT_CLASSES = {
	pink: "bg-accent-600 hover:bg-accent-700 text-white",
	black: "bg-stone-950 hover:bg-stone-800 text-white",
	bordered:
		"bg-white hover:bg-slate-200 border border-black text-black disabled:border-slate-500",
	transparent: "bg-transparent hover:font-bold hover:bg-stone-400/20 text-black"
} as const;

export type ButtonVariant = keyof typeof VARIANT_CLASSES;

export interface ButtonProps extends Omit<React.ComponentProps<"button">, "children" | "disabled"> {
	children?: React.ReactNode;
	variant?: ButtonVariant;
	icon?: string | null;
	full?: boolean;
	left?: boolean;
	iconClasses?: string;
	classes?: string;
	disabled?: boolean;
	sr?: string;
	href?: string | null;
}

export default function Button({
	children,
	variant,
	icon = null,
	full = false,
	left = false,
	iconClasses = "",
	classes = "",
	disabled = false,
	sr = "",
	href = null,
	...props
}: ButtonProps) {
	const className = cn(
		"flex items-center rounded-md",
		children !== undefined ? "px-4" : "px-2",
		"py-2 text-sm font-medium focus-visible:ring-4 focus-visible:outline-none focus-visible:ring-pink-300 transition-all",
		full && "w-full",
		left ? "justify-start" : "justify-center",
		disabled && "opacity-50 cursor-not-allowed",
		variant && VARIANT_CLASSES[variant],
		classes
	);

	const content = (
		<>
			{icon && (
				<span
					aria-hidden="true"
					className={`inline-flex items-start ${
						children !== undefined ? "w-4 mr-2" : "justify-center w-6 h-6"
					} ${iconClasses}`}
				>
					<ion-icon
						class={`hydrated ${children !== undefined ? "" : "w-6 h-6"}`}
						name={icon}
					/>
				</span>
			)}
			<span className="sr-only">{sr}</span>
			{children}
		</>
	);

	// Real anchors (not role="link" buttons): cmd-click, middle-click and
	// screen readers keep working. Never nest a button inside an anchor.
	if (href) {
		// the shared prop type is button-shaped; anchor-specific bits come
		// through the same spread (id, onClick, aria-*, ref, ...)
		const anchorProps = props as React.ComponentProps<"a">;
		return (
			<a href={href} className={className} {...anchorProps}>
				{content}
			</a>
		);
	}

	return (
		<button type="button" className={className} disabled={disabled} {...props}>
			{content}
		</button>
	);
}
