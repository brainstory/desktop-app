import * as React from "react";
import { cn } from "@helpers/cn";

const Input = React.forwardRef<
	HTMLInputElement,
	React.ComponentProps<"input">
>(({ className, type, ...props }, ref) => {
	return (
		<input
			type={type}
			className={cn(
				"border text-stone-900 text-sm rounded-lg border-stone-300",
				className
			)}
			ref={ref}
			{...props}
		/>
	);
});
Input.displayName = "Input";

export { Input };
