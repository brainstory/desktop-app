import { cloneElement, isValidElement, useId, type ReactElement, type ReactNode } from "react";

type Describable = ReactElement<{ "aria-describedby"?: string }>;

/**
 * Hover/focus tooltip. When the child is a single element (the usual
 * case: a button or link), the tooltip id is merged into THAT element's
 * aria-describedby so the focusable trigger itself is described; the
 * tooltip shows on hover and on keyboard focus within.
 */
export default function Tooltip({
	children,
	text,
	position,
	classes = ""
}: {
	children: ReactNode;
	text?: string;
	position?: string;
	classes?: string;
}) {
	let positionClasses = "left-1/2 -translate-x-1/2";
	if (position) {
		positionClasses = position;
	}

	// named group: an enclosing `group` (e.g. a hoverable card) must not
	// reveal every tooltip inside it
	const tooltipClass = `${positionClasses} pointer-events-none group-hover/tooltip:opacity-100 group-focus-within/tooltip:opacity-100 transition-opacity bg-stone-800 p-2 px-4 text-xs text-white rounded-md absolute translate-y-10 opacity-0 z-50 text-center`;
	const containerClass = `group/tooltip flex relative ${classes}`;
	const tooltipId = useId();

	let trigger = children;
	if (text && isValidElement(children)) {
		const child = children as Describable;
		const describedBy = [child.props["aria-describedby"], tooltipId].filter(Boolean).join(" ");
		trigger = cloneElement(child, { "aria-describedby": describedBy });
	}

	return (
		<div className={containerClass}>
			{trigger}
			{text && (
				<span id={tooltipId} role="tooltip" className={tooltipClass}>
					{text}
				</span>
			)}
		</div>
	);
}
