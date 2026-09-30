import type { Dispatch, SetStateAction } from "react";
import { CHAT_SAVE_STATE } from "@src/const";

import TransparentButton from "@ds/TransparentButton";
import ChatIntroText from "@components/chat/ChatIntroText";

interface ChatTopBarProps {
	parentIdea?: { title?: string | null } | null;
	showTranscript: boolean;
	setShowTranscript: Dispatch<SetStateAction<boolean>>;
	leftButtonIcon?: string | null;
	leftButtonHref?: string;
	saveState: string;
}

export default function ChatTopBar({
	parentIdea,
	showTranscript,
	setShowTranscript,
	leftButtonIcon,
	leftButtonHref,
	saveState
}: ChatTopBarProps) {
	const isLeftButton = leftButtonHref && leftButtonIcon;

	// derived from saveState; no effect/state needed
	const saveIconName =
		saveState === CHAT_SAVE_STATE.SAVING
			? "sync-outline" //reload-circle-outline
			: saveState === CHAT_SAVE_STATE.SUCCESS
				? "checkmark-outline"
				: saveState === CHAT_SAVE_STATE.FAILED
					? "close-outline"
					: undefined;

	const renderLeftComponent = () => {
		// The guide-flow back button must survive while a save is in
		// flight - it used to be replaced by the save indicator, stranding
		// users mid-onboarding. Both render side by side now.
		return (
			<div className="flex items-center flex-nowrap gap-2">
				{isLeftButton && (
					<TransparentButton
						icon={leftButtonIcon}
						href={leftButtonHref}
						sr="Go back"
						classes="order-first"
					/>
				)}
				{saveIconName && (
					<div role="status" className="flex items-center flex-nowrap gap-1 text-sm">
						<ion-icon
							name={saveIconName}
							class="hydrated w-5 h-5 text-stone-500"
						></ion-icon>
						<p className="text-stone-500">{saveState}</p>
					</div>
				)}
				{!isLeftButton && !saveIconName && <div />}
			</div>
		);
	};

	return (
		<div
			className={`flex justify-between z-10 w-full p-4 bg-white border-b border-stone-200 rounded-t-lg ${!parentIdea && "sm:grid sm:grid-cols-3"}`}
		>
			{renderLeftComponent()}
			<ChatIntroText
				parentIdea={parentIdea}
				classes={`sm:block ${(!parentIdea || saveIconName) && "hidden"} text-center grow`}
			/>
			{!parentIdea && (
				<TransparentButton
					classes="border border-stone-200 order-last ml-auto text-nowrap"
					aria-pressed={showTranscript}
					onClick={() => {
						setShowTranscript(!showTranscript);
					}}
				>
					{showTranscript ? "Hide Transcript" : "Show Transcript"}
				</TransparentButton>
			)}
		</div>
	);
}
