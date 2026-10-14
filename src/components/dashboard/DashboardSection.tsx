import type { IdeaListItem } from "@src/types";
import { useState, useEffect } from "react";
import { getAllIdeasApi } from "@helpers/api/user";
import { importShareApi, type ImportShareResult } from "@helpers/api/share";
import { normalizeApiError } from "@helpers/helpers";
import { setGettingStartedDone } from "@helpers/storage";
import { isOwnIdea } from "@helpers/ideas";

import LoadingAnimation from "@components/global/LoadingAnimation";
import Button from "@ds/Button";
import { useSnackbar } from "@ds/Snackbar";
import ErrorSection from "@components/error/ErrorSection";
import ErrorBoundary from "@components/global/ErrorBoundary";

import IdeaGrid from "./IdeaGrid";
import FeedbackGrid from "./FeedbackGrid";
import AiSetupNeeded from "./AiSetupNeeded";
import { useLibraryView } from "./LibraryToolbar";

function DashboardSectionInner() {
	const [userIdeas, setUserIdeas] = useState<IdeaListItem[] | undefined>(undefined);
	const [isLoading, setIsLoading] = useState(true);
	/** a post-import refresh is in flight: dim the grid, keep it mounted */
	const [isRefreshing, setIsRefreshing] = useState(false);
	const [errorFound, setErrorFound] = useState(false);
	const { openSnackbar, snackbars } = useSnackbar();

	useEffect(() => {
		getAllIdeasApi()
			.then((ideas) => {
				setUserIdeas(ideas);
			})
			.catch((err) => {
				// surface the failure instead of falling through to the
				// first-run empty state, which would mislead the user
				console.error("error getting ideas data", err);
				setErrorFound(true);
			})
			.finally(() => setIsLoading(false));
	}, []);

	// derived during render
	// An "own" idea is one without creator attribution (imports carry
	// creatorName) - isOwnIdea, shared with the idea page. The library
	// grid shows whenever ANY idea exists: an imported-only library is a
	// library too.
	const userIdeasList = userIdeas ?? [];
	const hasOwnIdea = userIdeasList.some(isOwnIdea);
	const showGetStarted = userIdeasList.length === 0;
	// search/filter/sort over the already-loaded library
	const library = useLibraryView(userIdeasList);

	useEffect(() => {
		if (userIdeas && hasOwnIdea) {
			// keep the onboarding flag in sync with reality (e.g. for past
			// users who already created ideas before /get-started existed).
			// Monotonic: deleting every own idea must not resurrect
			// onboarding.
			setGettingStartedDone(true);
		}
	}, [userIdeas, hasOwnIdea]);

	/** Where an import's "Open" action leads: the idea itself, or the
	 * feedback tab of its parent. */
	const importedHref = (res: ImportShareResult): string | undefined => {
		if (!res.id) return undefined;
		if (res.kind === "feedback" && res.parent_id) {
			return `/idea?id=${res.parent_id}&tab=feedback`;
		}
		return `/idea?id=${res.id}`;
	};

	const refreshAfterImport = () => {
		// keep the grid mounted (its scroll and card state survive); only
		// dim it while the fresh list arrives
		setIsRefreshing(true);
		getAllIdeasApi()
			.then((ideas) => {
				setUserIdeas(ideas);
				setErrorFound(false);
			})
			.catch((err) => {
				console.error("refresh after import failed", err);
				setErrorFound(true);
			})
			.finally(() => setIsRefreshing(false));
	};

	const handleImport = () => {
		importShareApi()
			.then((res) => {
				if (res.cancelled) return;
				const href = importedHref(res);
				if (res.duplicate) {
					openSnackbar(
						true,
						"Already in your library",
						href ? { label: "Open", href } : undefined
					);
				} else {
					openSnackbar(
						true,
						res.kind === "feedback"
							? `Imported feedback from ${res.author}`
							: `Imported "${res.title}" from ${res.author}`,
						href ? { label: "Open", href } : undefined
					);
				}
				refreshAfterImport();
			})
			.catch((err) => {
				openSnackbar(false, normalizeApiError(err));
			});
	};

	return (
		<section>
			{snackbars}
			<div className="flex items-center justify-center relative">
				<h1 className="mb-2 text-2xl font-bold tracking-tight text-center text-stone-900 md:text-2xl lg:text-4xl">
					Dashboard
				</h1>
				<Button
					variant="transparent"
					icon="download-outline"
					onClick={handleImport}
					classes="absolute right-0 top-0"
					aria-label="Import shared idea or feedback"
				>
					Import
				</Button>
			</div>
			<AiSetupNeeded />
			{errorFound ? (
				<ErrorSection
					title="Couldn't load your ideas"
					paragraphs={["Something went wrong while loading your library."]}
					action={
						<Button variant="pink" onClick={() => window.location.reload()}>
							Try again
						</Button>
					}
					hideDashboardLink
				/>
			) : isLoading ? (
				<LoadingAnimation />
			) : showGetStarted ? (
				<div className="flex flex-col-reverse sm:flex-col gap-8 justify-between items-center my-6">
					<div className="">
						{/* one asset per viewport: <picture> keeps the other
						    file off the wire entirely (CSS alone downloaded
						    both and hid one) */}
						<picture>
							<source media="(max-width: 639px)" srcSet="/comic2-2.png" />
							<img
								src="/comic1-4.png"
								className="pointer-events-none pb-1"
								alt="Comic: someone unsure how to start, encouraged to think out loud"
							/>
						</picture>

						<div>
							<p className="text-xs text-right text-stone-600 italic">
								Art inspired by &lsquo;Oh No Comics&rsquo; by Alex Norris
							</p>
						</div>
					</div>
					<Button
						classes="bg-stone-200 hover:bg-stone-300 shadow-lg border border-stone-600 rounded-2xl"
						href="/get-started"
					>
						<div className="w-56 p-6 flex flex-col items-center gap-2">
							{/* decorative: the button's text names the action */}
							<ion-icon
								class="hydrated w-16 h-16"
								name="mic-outline"
								aria-hidden="true"
							></ion-icon>
							<p>Find a quiet place</p>
							<p className="text-xl font-bold">Start your first Brainstory!</p>
						</div>
					</Button>
				</div>
			) : (
				<div>
					<FeedbackGrid />
					{library.toolbar}
					{library.isFiltering && library.visible.length === 0 ? (
						<div className="text-center py-10">
							<p className="text-stone-600 mb-3">
								No ideas match your search or filters.
							</p>
							<Button variant="bordered" onClick={library.clear}>
								Clear filters
							</Button>
						</div>
					) : (
						<div className={isRefreshing ? "opacity-50 transition-opacity" : undefined}>
							<IdeaGrid userIdeas={library.visible} />
						</div>
					)}
				</div>
			)}
		</section>
	);
}

/** The dashboard island: a crash shows a recoverable section, not a blank page. */
export default function DashboardSection() {
	return (
		<ErrorBoundary title="The dashboard hit a snag">
			<DashboardSectionInner />
		</ErrorBoundary>
	);
}
