import { useState, useEffect } from "react";

import { getUserDailyStatusApi } from "@helpers/api/user";

export default function DailyStreakSection() {
	const [streakCount, setStreakCount] = useState(0);
	const [intentIdeaId, setIntentIdeaId] = useState<string | undefined>(undefined); // if isTodayIntentCompleted is false, this is a draft
	const [isTodayIntentCompleted, setIsTodayIntentCompleted] = useState(false);
	const [isLoaded, setIsLoaded] = useState(false);

	useEffect(() => {
		getUserDailyStatusApi()
			.then((resp) => {
				setStreakCount(resp.streak);
				setIntentIdeaId(resp.intentIdeaId ?? undefined);
				setIsTodayIntentCompleted(resp.isCompleted);
				setIsLoaded(true);
			})
			.catch((err) => {
				console.error("failed to load daily status", err);
				setIsLoaded(true);
			});
	}, []);

	const renderTodayStatus = () => {
		// Descriptive link text (never a bare "here")
		if (isTodayIntentCompleted) {
			return (
				<div className="text-start text-sm sm:text-base text-stone-700">
					<p className="font-semibold tracking-wide text-stone-900">Great job!</p>
					<p>
						You set today&rsquo;s intent.{" "}
						<a
							className="text-accent-900 font-medium hover:underline"
							href={`/idea?id=${intentIdeaId}`}
						>
							See today&rsquo;s idea
						</a>
					</p>
				</div>
			);
		}

		if (intentIdeaId) {
			return (
				<div className="text-start text-sm sm:text-base text-stone-700">
					<p className="font-semibold tracking-wide text-stone-900">
						{streakCount > 0 ? "Keep that streak going!" : "Almost there!"}
					</p>
					<p>
						You&rsquo;re partway through.{" "}
						<a
							className="text-accent-900 font-medium hover:underline"
							href={`/chat?dailyIntent=true&id=${intentIdeaId}`}
						>
							Finish today&rsquo;s daily intent
						</a>
					</p>
				</div>
			);
		}

		return (
			<div className="text-start text-sm sm:text-base text-stone-700">
				<p className="font-semibold tracking-wide text-stone-900">Start your streak!</p>
				<p>
					<a
						className="text-accent-900 font-medium hover:underline"
						href="/chat?dailyIntent=true"
					>
						Do today&rsquo;s daily intent
					</a>
				</p>
			</div>
		);
	};
	if (!isLoaded) {
		// pulsing skeleton: don't flash "0 days" before the data arrives
		return (
			<section
				aria-busy="true"
				className="flex flex-wrap lg:flex-nowrap gap-4 lg:gap-0 justify-evenly items-center p-5 rounded-t-lg animate-pulse"
			>
				<div className="text-start">
					<div className="h-6 w-16 bg-stone-200 rounded" />
					<div className="h-3 w-10 bg-stone-100 rounded mt-1" />
				</div>
				<div className="h-4 w-48 bg-stone-200 rounded" />
			</section>
		);
	}

	return (
		<section className="flex flex-wrap lg:flex-nowrap gap-4 lg:gap-0 justify-evenly items-center p-5 rounded-t-lg">
			<div className="text-start">
				<div className="flex gap-1 items-center">
					<ion-icon
						class="w-6 h-6 hydrated pointer-events-none fill-red-500"
						name="flame"
					></ion-icon>
					<p className="font-bold tracking-wide text-xl text-nowrap">
						{streakCount} {streakCount === 1 ? "day" : "days"}
					</p>
				</div>
				<p className="text-stone-500 text-sm text-nowrap">Streak</p>
			</div>
			<div className="hidden lg:block h-8 w-[1px] bg-stone-400" />
			<div className="">{renderTodayStatus()}</div>
		</section>
	);
}
