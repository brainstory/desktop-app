import type { ReactNode } from "react";

interface ErrorSectionProps {
	title: string;
	paragraphs?: string[];
	/** Optional custom action (e.g. a retry button) rendered under the text. */
	action?: ReactNode;
	/** Show the "try restarting the app" hint; off by default since most
	 * errors here are "not found", where it reads as noise. */
	showRestartHint?: boolean;
	/** Hide the "Back to dashboard" link (e.g. when already there). */
	hideDashboardLink?: boolean;
}

export default function ErrorSection({
	title,
	paragraphs = [],
	action,
	showRestartHint = false,
	hideDashboardLink = false
}: ErrorSectionProps) {
	return (
		<div className="mx-auto w-full px-6 md:px-24 max-w-4xl py-12 scroll-mt-12">
			<p className="text-black font-bold lg:text-5xl text-4xl tracking-tight">{title}</p>
			<div className="text-stone-500 lg:text-xl text-base flex flex-col gap-3 mt-6">
				{paragraphs.map((pText, i) => {
					return <p key={i}>{pText}</p>;
				})}
				{showRestartHint && <p>If you keep seeing this page, try restarting the app.</p>}
			</div>
			{(action || !hideDashboardLink) && (
				<div className="flex flex-wrap gap-3 mt-8">
					{action}
					{!hideDashboardLink && (
						<a
							className="text-sm font-semibold text-accent-800 underline underline-offset-2 hover:no-underline"
							href="/dashboard"
						>
							Back to dashboard
						</a>
					)}
				</div>
			)}
		</div>
	);
}
