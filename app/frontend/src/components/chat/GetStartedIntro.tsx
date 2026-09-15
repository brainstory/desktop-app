import { useState, useEffect } from "react";
import { hasDoneGettingStarted } from "@helpers/storage";
import { TOPICS } from "@src/const";

import { useAppContext } from "@components/chat/reusable/AppWrapper";

import RivePencil from "@components/global/RivePencil";
import TextsFadeIn from "./reusable/TextsFadeIn";

export default function GetStartedIntro() {
	const [isTextLoading, setIsTextLoading] = useState(true);
	const { sludgeman } = useAppContext();

	const hasDone = hasDoneGettingStarted();

	let introTextComponents;
	if (hasDone) {
		introTextComponents = [
			<p key="intro-hi" className="text-accent-900 font-semibold">
				Hey there!{" "}
				<span className="hidden lg:inline">It&rsquo;s your favorite pencil, Mark.</span>
			</p>,
			<p key="intro-inspire">
				Need some inspiration? Here are some things we can start with! Do you want to:
			</p>
		];
	} else {
		introTextComponents = [
			<p key="intro-hi" className="text-accent-900 font-semibold">
				Hey there! <span className="hidden lg:inline">My name is Mark.</span>
			</p>,
			<p key="intro-welcome">
				Welcome to Brainstory, your{" "}
				<span className="underline decoration-accent-900 inline font-semibold">
					think-out-loud tool
				</span>{" "}
				for quick insights and creative boosts.
			</p>,
			<p key="intro-start">Here are some things we can start with! Do you want to:</p>
		];
	}

	useEffect(() => {
		const timer = setTimeout(() => {
			setIsTextLoading(false);
		}, 1800);
		return () => clearTimeout(timer);
	}, []);

	return (
		<div className="p-6 md:p-10">
			<div className="flex flex-col md:flex-row justify-center items-center">
				<div className="sludge-sludge-maaaan hidden lg:block lg:mr-4">
					{sludgeman == "idle" ? (
						<RivePencil type="wave" small={false} />
					) : (
						<RivePencil type="jump" small={false} />
					)}
				</div>
				<TextsFadeIn classes="tracking-tight text-xl md:text-2xl max-w-2xl flex flex-col gap-3">
					{introTextComponents}
				</TextsFadeIn>
			</div>
			<div className="flex flex-wrap gap-3 max-w-7xl justify-center mx-auto my-6">
				{!isTextLoading &&
					TOPICS.map((topic, index) => (
						<a
							key={`topic-${topic}-${index}`}
							className="p-6 font-semibold flex flex-col gap-3 justify-center items-center animate-appear block w-72 bg-white border border-stone-200 text-center cursor-pointer rounded-lg shadow hover:shadow-lg hover:-translate-y-1 transition-transform min-h-[144px]"
							href={`/chat?topic=${index}`}
						>
							<ion-icon name={topic.iconName} class="hydrated w-8 h-8" />
							{topic.topic}
						</a>
					))}
			</div>
		</div>
	);
}
