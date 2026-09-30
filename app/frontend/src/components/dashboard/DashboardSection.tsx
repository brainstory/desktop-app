import type { IdeaListItem } from "@src/types";
import { useState, useEffect } from "react";
import { getAllIdeasApi } from "@helpers/api/user";
import { importShareApi } from "@helpers/api/share";
import { setGettingStartedDone } from "@helpers/storage";

import LoadingAnimation from "@components/global/LoadingAnimation";
import Button from "@ds/Button";
import TransparentButton from "@ds/TransparentButton";
import PinkButton from "@ds/PinkButton";
import { Snackbar, ERROR_COPY, SUCCESS_COPY } from "@ds/Snackbar";
import ErrorSection from "@components/error/ErrorSection";

import IdeaGrid from "./IdeaGrid";
import FeedbackGrid from "./FeedbackGrid";
import AiSetupNeeded from "./AiSetupNeeded";

export default function DashboardSection() {
	const [userIdeas, setUserIdeas] = useState<IdeaListItem[] | undefined>(undefined);
	const [isLoading, setIsLoading] = useState(true);
	const [errorFound, setErrorFound] = useState(false);
	const [snackbarSuccessOpen, setSnackbarSuccessOpen] = useState(false);
	const [snackbarSuccessMessage, setSnackbarSuccessMessage] = useState(SUCCESS_COPY.DEFAULT);
	const [snackbarErrorOpen, setSnackbarErrorOpen] = useState(false);
	const [snackbarErrorMessage, setSnackbarErrorMessage] = useState(ERROR_COPY.DEFAULT);

	useEffect(() => {
		getAllIdeasApi()
			.then((ideas) => {
				setUserIdeas(ideas);
			})
			.catch((err) => {
				// surface the failure instead of falling through to the
				// first-run empty state, which would mislead the user
				console.log("error getting ideas data", err);
				setErrorFound(true);
			})
			.finally(() => setIsLoading(false));
	}, []);

	// derived during render
	// An "own" idea is one without creator attribution (imports carry
	// creatorName) - the same definition the idea page uses. The library
	// grid shows whenever ANY idea exists: an imported-only library is a
	// library too.
	const userIdeasList = userIdeas ?? [];
	const hasOwnIdea = userIdeasList.some((idea) => !idea.creatorName);
	const showGetStarted = userIdeasList.length === 0;

	useEffect(() => {
		if (userIdeas && hasOwnIdea) {
			// keep the onboarding flag in sync with reality (e.g. for past
			// users who already created ideas before /get-started existed).
			// Monotonic: deleting every own idea must not resurrect
			// onboarding.
			setGettingStartedDone(true);
		}
	}, [userIdeas, hasOwnIdea]);

	const handleImport = () => {
		importShareApi()
			.then((res) => {
				if (res.cancelled) return;
				setSnackbarSuccessOpen(true);
				setSnackbarSuccessMessage(
					res.kind === "feedback"
						? `Imported feedback from ${res.author}`
						: `Imported "${res.title}" from ${res.author}`
				);
				setIsLoading(true);
				getAllIdeasApi()
					.then((ideas) => {
						setUserIdeas(ideas);
						setErrorFound(false);
					})
					.catch((err) => {
						console.error("refresh after import failed", err);
						setErrorFound(true);
					})
					.finally(() => setIsLoading(false));
			})
			.catch((err) => {
				setSnackbarErrorOpen(true);
				setSnackbarErrorMessage(err);
			});
	};

	return (
		<section>
			{snackbarSuccessOpen && (
				<Snackbar
					isSuccess={true}
					message={snackbarSuccessMessage}
					onClose={() => setSnackbarSuccessOpen(false)}
				/>
			)}
			{snackbarErrorOpen && (
				<Snackbar
					isSuccess={false}
					message={snackbarErrorMessage}
					onClose={() => setSnackbarErrorOpen(false)}
				/>
			)}
			<div className="flex items-center justify-center relative">
				<h1 className="mb-2 text-2xl font-bold tracking-tight text-center text-stone-900 md:text-2xl lg:text-4xl">
					Dashboard
				</h1>
				<TransparentButton
					icon="download-outline"
					onClick={handleImport}
					classes="absolute right-0 top-0"
					aria-label="Import shared idea or feedback"
				>
					Import
				</TransparentButton>
			</div>
			<AiSetupNeeded />
			{errorFound ? (
				<ErrorSection
					title="Couldn't load your ideas"
					paragraphs={["Something went wrong while loading your library."]}
					action={
						<PinkButton onClick={() => window.location.reload()}>Try again</PinkButton>
					}
					hideDashboardLink
				/>
			) : isLoading ? (
				<LoadingAnimation />
			) : showGetStarted ? (
				<div className="flex flex-col-reverse sm:flex-col gap-8 justify-between items-center my-6">
					<div className="">
						{/* CSS-driven art swap so it tracks window resizes */}
						<img
							src="/comic2-2.png"
							className="pointer-events-none pb-1 sm:hidden"
							alt="Comic: someone unsure how to start, encouraged to think out loud"
						/>
						<img
							src="/comic1-4.png"
							className="pointer-events-none pb-1 hidden sm:block"
							alt="Comic: someone unsure how to start, encouraged to think out loud"
						/>

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
							<ion-icon
								class="hydrated w-16 h-16"
								name="mic-outline"
								role="img"
							/>
							<p>Find a quiet place</p>
							<p className="text-xl font-bold">Start your first Brainstory!</p>
						</div>
					</Button>
				</div>
			) : (
				<div>
					<FeedbackGrid />
					<IdeaGrid userIdeas={userIdeas} />
				</div>
			)}
		</section>
	);
}
