interface TextsFadeInProps {
	children: ReactNode[];
	classes?: string;
}

import type { ReactNode } from "react";

export default function TextsFadeIn({ children, classes }: TextsFadeInProps) {
	// global.css defines appear0..appear6; wrap around instead of
	// emitting an animation class that doesn't exist (appear7 never
	// animated anything)
	const animateClass = [
		"animate-appear0",
		"animate-appear1",
		"animate-appear2",
		"animate-appear3",
		"animate-appear4",
		"animate-appear5",
		"animate-appear6"
	];

	const fadeClasses = children.map(
		(_: unknown, i: number) => `${animateClass[i % animateClass.length]} opacity-0`
	);

	return (
		<div className={classes}>
			{children.map((child: ReactNode, i: number) => (
				<span key={i} className={fadeClasses[i]}>
					{child}
				</span>
			))}
		</div>
	);
}
