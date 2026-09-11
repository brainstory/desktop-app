import { useState, useEffect } from "react";
import { getAllIdeasApi } from "@helpers/api/user.js";
import { importShareApi } from "@helpers/api/share.js";
import { setGettingStartedDone } from "@helpers/storage";

import LoadingAnimation from "@components/global/LoadingAnimation";
import Button from "@ds/Button";
import TransparentButton from "@ds/TransparentButton";
import { Snackbar, ERROR_COPY, SUCCESS_COPY } from "@ds/Snackbar";

import IdeaGrid from "./IdeaGrid.jsx";
import FeedbackGrid from "./FeedbackGrid.jsx";
import AiSetupNeeded from "./AiSetupNeeded.jsx";

export default function DashboardSection() {
	let [userIdeas, setUserIdeas] = useState();
	let [isLoading, setIsLoading] = useState(true);
	let [showGetStarted, setShowGetStarted] = useState(false);
	let [snackbarSuccessOpen, setSnackbarSuccessOpen] = useState(false);
	let [snackbarSuccessMessage, setSnackbarSuccessMessage] = useState(SUCCESS_COPY.DEFAULT);
	let [snackbarErrorOpen, setSnackbarErrorOpen] = useState(false);
	let [snackbarErrorMessage, setSnackbarErrorMessage] = useState(ERROR_COPY.DEFAULT);

	useEffect(() => {
		getAllIdeasApi()
			.then((ideas) => {
				setUserIdeas(ideas);
			})
			.catch((err) => console.log("error getting ideas data", err))
			.finally(() => setIsLoading(false));
	}, []);

	useEffect(() => {
		if (userIdeas) {
			const hasCreatedIdea =
				userIdeas.reduce((acc, idea) => acc || !idea.creatorEmail, false) || false;
			setShowGetStarted(!hasCreatedIdea);

			// keep the onboarding flag in sync with reality (e.g. for past
			// users who already created ideas before /get-started existed)
			setGettingStartedDone(hasCreatedIdea);
		}
	}, [userIdeas]);

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
				setTimeout(() => setSnackbarSuccessOpen(false), 5000);
				setIsLoading(true);
				getAllIdeasApi()
					.then((ideas) => setUserIdeas(ideas))
					.finally(() => setIsLoading(false));
			})
			.catch((err) => {
				setSnackbarErrorOpen(true);
				setSnackbarErrorMessage(err);
				setTimeout(() => setSnackbarErrorOpen(false), 5000);
			});
	};

	return (
		<section>
			{snackbarSuccessOpen && <Snackbar isSuccess={true} message={snackbarSuccessMessage} closeAfterTime={() => setSnackbarSuccessOpen(false)} />}
			{snackbarErrorOpen && <Snackbar isSuccess={false} message={snackbarErrorMessage} closeAfterTime={() => setSnackbarErrorOpen(false)} />}
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
			{isLoading ? (
				<LoadingAnimation />
			) : showGetStarted ? (
				<div className="flex flex-col-reverse sm:flex-col gap-8 justify-between items-center my-6">
					<div className="">
						<img
							src={window.innerWidth <= 768 ? "/comic2-2.png" : "/comic1-4.png"}
							className="pointer-events-none pb-1"
							alt="comic inspired by oh no"
						/>

						<div>
							<p className="text-xs text-right text-stone-600 italic">
								Art inspired by 'Oh No Comics' by Alex Norris
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
							<p className="text-xl font-bold">
								Tap here for your first Brainstory!
							</p>
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
