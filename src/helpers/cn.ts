/**
 * Join class names, skipping falsy values. Local replacement for the
 * shadcn-style `cn` helper so the design system has no clsx/tailwind-merge
 * dependency (the call sites only concatenate static + optional classes).
 */
export function cn(...inputs: unknown[]): string {
	return inputs.filter(Boolean).join(" ");
}
