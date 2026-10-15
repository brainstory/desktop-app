import React from "react";
import { cn } from "@helpers/cn";
import { safeHref } from "@helpers/helpers";
import Icon from "./Icon";

/** Colour schemes (formerly PinkButton / BlackButton / BorderedButton /
 * TransparentButton). Without a variant the button is uncoloured. */
const VARIANT_CLASSES = {
	pink: "bg-accent-600 hover:bg-accent-700 text-white",
	black: "bg-stone-950 hover:bg-stone-800 text-white",
	// an inset ring, not a border: a border adds 2px, so a bordered
	// button sat taller than the filled ones next to it
	bordered:
		"bg-white hover:bg-slate-200 ring-1 ring-inset ring-black text-black disabled:ring-slate-500",
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
					{/* explicit size: an svg with only a viewBox and no
					 * width/height collapses in the webview */}
					<Icon
						name={icon}
						className={children !== undefined ? "w-4 h-4" : "w-full h-full"}
					/>
				</span>
			)}
			<span className="sr-only">{sr}</span>
			{children}
		</>
	);

	// Real anchors (not role="link" buttons): cmd-click, middle-click and
	// screen readers keep working. Never nest a button inside an anchor.
	// The href reaches a real navigation surface, so it goes through the
	// same-app/http(s)-only guard: anything else degrades to the button
	// below instead of navigating (and says so in the console).
	const url = href ? safeHref(href) : null;
	if (href && url === null) {
		console.error(`Button: refusing unsafe href ${JSON.stringify(href)}`);
	}
	if (url) {
		// the shared prop type is button-shaped; anchor-specific bits come
		// through the same spread (id, onClick, aria-*, ref, ...)
		const anchorProps = props as React.ComponentProps<"a">;
		return (
			// the taint tracker cannot see through safeHref (a validating
			// helper is never a sanitizer to it); the href here is only
			// ever a same-app path or an absolute http(s) URL - see
			// safeHref's tests
			// codeql[js/client-side-unvalidated-url-redirection]
			<a href={url} className={className} {...anchorProps}>
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
