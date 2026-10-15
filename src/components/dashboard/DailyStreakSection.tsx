import { useState, useEffect } from "react";

import { getUserDailyStatusApi } from "@helpers/api/user";
import Button from "@ds/Button";
import Icon from "@ds/Icon";

/**
 * The daily intent streak: consecutive days the daily intent flow was
 * used (a log submitted, an intent started, or one completed). The copy
 * names the streak after the loop it measures, and a failed load says so
 * instead of masquerading as a zero streak.
 */
export default function DailyStreakSection() {
	const [streakCount, setStreakCount] = useState(0);
	const [intentIdeaId, setIntentIdeaId] = useState<string | undefined>(undefined); // if isTodayIntentCompleted is false, this is a draft
	const [isTodayIntentCompleted, setIsTodayIntentCompleted] = useState(false);
	const [hasLogToday, setHasLogToday] = useState(false);
	const [isLoaded, setIsLoaded] = useState(false);
	const [loadError, setLoadError] = useState(false);
	const [loadAttempt, setLoadAttempt] = useState(0);

	useEffect(() => {
		let isCurrent = true;
		getUserDailyStatusApi()
			.then((resp) => {
				if (!isCurrent) return;
				setStreakCount(resp.streak);
				setIntentIdeaId(resp.intentIdeaId ?? undefined);
				setIsTodayIntentCompleted(resp.isCompleted);
				setHasLogToday(Boolean(resp.logId));
				setLoadError(false);
				setIsLoaded(true);
			})
			.catch((err) => {
				if (!isCurrent) return;
				console.error("failed to load daily status", err);
				setLoadError(true);
				setIsLoaded(true);
			});
		return () => {
			isCurrent = false;
		};
	}, [loadAttempt]);

	const retry = () => {
		setIsLoaded(false);
		setLoadAttempt((attempt) => attempt + 1);
	};

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

		if (hasLogToday) {
			return (
				<div className="text-start text-sm sm:text-base text-stone-700">
					<p className="font-semibold tracking-wide text-stone-900">Today counts!</p>
					<p>
						Your daily log is in.{" "}
						<a
							className="text-accent-900 font-medium hover:underline"
							href="/chat?dailyIntent=true"
						>
							Think out loud about your day too
						</a>
					</p>
				</div>
			);
		}

		if (streakCount > 0) {
			// the streak is alive but today is untouched: say what keeps it
			return (
				<div className="text-start text-sm sm:text-base text-stone-700">
					<p className="font-semibold tracking-wide text-stone-900">
						Keep that streak going!
					</p>
					<p>
						{`Do today’s daily intent to keep your ${streakCount}-day streak. `}
						<a
							className="text-accent-900 font-medium hover:underline"
							href="/chat?dailyIntent=true"
						>
							Do today&rsquo;s daily intent
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

	if (loadError) {
		// a failed load is not a zero streak: say so and offer a retry
		return (
			<section className="flex flex-wrap lg:flex-nowrap gap-4 lg:gap-0 justify-evenly items-center p-5 rounded-t-lg">
				<div className="text-start">
					<div className="flex gap-1 items-center">
						<Icon name="flame" className="w-6 h-6 pointer-events-none fill-stone-400" />
						<p className="font-bold tracking-wide text-xl text-nowrap text-stone-400">
							—
						</p>
					</div>
					<p className="text-stone-500 text-sm text-nowrap">Daily intent streak</p>
				</div>
				<div className="hidden lg:block h-8 w-[1px] bg-stone-400" />
				<div
					role="alert"
					className="flex items-center gap-3 text-start text-sm text-stone-700"
				>
					<p>Couldn&rsquo;t load your streak.</p>
					<Button variant="bordered" onClick={retry}>
						Try again
					</Button>
				</div>
			</section>
		);
	}

	return (
		<section className="flex flex-wrap lg:flex-nowrap gap-4 lg:gap-0 justify-evenly items-center p-5 rounded-t-lg">
			<div className="text-start">
				<div className="flex gap-1 items-center">
					<Icon name="flame" className="w-6 h-6 pointer-events-none fill-red-500" />
					<p className="font-bold tracking-wide text-xl text-nowrap">
						{streakCount} {streakCount === 1 ? "day" : "days"}
					</p>
				</div>
				<p className="text-stone-500 text-sm text-nowrap">Daily intent streak</p>
			</div>
			<div className="hidden lg:block h-8 w-[1px] bg-stone-400" />
			<div className="">{renderTodayStatus()}</div>
		</section>
	);
}
