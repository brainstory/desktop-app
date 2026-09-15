interface TextsFadeInProps {
	children: ReactNode[];
	classes?: string;
}

import type { ReactNode } from "react";

export default function TextsFadeIn({ children, classes }: TextsFadeInProps) {
	// very manual mapping of animation names to various delays
	const animateClass = [
		"animate-appear0",
		"animate-appear1",
		"animate-appear2",
		"animate-appear3",
		"animate-appear4",
		"animate-appear5",
		"animate-appear6",
		"animate-appear7"
	];

	const fadeClasses = children.map((_: unknown, i: number) => `${animateClass[i]} opacity-0`);

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
