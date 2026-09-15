/// JSX support for the ionicons web components (used as <ion-icon />).
import "react";

declare module "react" {
	namespace JSX {
		interface IntrinsicElements {
			"ion-icon": React.DetailedHTMLProps<
				React.HTMLAttributes<HTMLElement>,
				HTMLElement
			> & {
				/** icon name, e.g. "mic" or "arrow-back-outline" */
				name?: string;
				/** web components read `class`, not React's `className` */
				class?: string;
				size?: string;
				color?: string;
			};
		}
	}
}
