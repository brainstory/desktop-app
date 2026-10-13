import { useState, useEffect } from "react";

import { getUserDailyStatusApi } from "@helpers/api/user";
import { getQueryParam } from "@helpers/helpers";
import { QUERY_PARAMS } from "@src/tauri/commands";
import { CHAT_TYPE, type ChatType } from "@src/const";

import { ChatSection } from "./ChatSection";
import { AppWrapper } from "@components/chat/reusable/AppWrapper";
import DailyIntentModal from "@components/form/DailyIntentModal";
import AiSetupNeeded from "@components/dashboard/AiSetupNeeded";

/** Resolve the chat mode from the URL once, inside the component (module
 * scope would read window.location at import time and make the module
 * untestable). */
function chatTypeFromParams(): ChatType {
	const parentId = getQueryParam(QUERY_PARAMS.parentId);
	if (getQueryParam(QUERY_PARAMS.dailyIntent) === "true") {
		return CHAT_TYPE.DAILY_INTENT;
	}
	if (parentId) {
		return CHAT_TYPE.FEEDBACK;
	}
	return CHAT_TYPE.ORIGINAL;
}

export default function ChatApp() {
	const isDailyIntent = getQueryParam(QUERY_PARAMS.dailyIntent) === "true";
	const chatType = chatTypeFromParams();
	const parentId = getQueryParam(QUERY_PARAMS.parentId);
	const isFromGuide = getQueryParam(QUERY_PARAMS.topic);
	const [logId, setLogId] = useState<string | null | undefined>(null);
	const [draftId, setDraftId] = useState(getQueryParam(QUERY_PARAMS.id));
	const [isLogModalOpen, setIsLogModalOpen] = useState(false);
	/** the daily-status read failed: warn + offer a retry instead of
	 * silently leaving the session unlinked from today's log */
	const [dailyStatusError, setDailyStatusError] = useState(false);
	const [statusAttempt, setStatusAttempt] = useState(0);

	useEffect(() => {
		if (isDailyIntent) {
			let isCurrent = true;
			getUserDailyStatusApi()
				.then((resp) => {
					if (!isCurrent) return;
					setDailyStatusError(false);
					if (!resp.logId) {
						setIsLogModalOpen(true);
					}
					setLogId(resp.logId);
					if (resp.intentIdeaId) {
						// intent idea draft is found
						const url = new URL(window.location.href);
						const params = new URLSearchParams(url.search);
						params.set(QUERY_PARAMS.id, resp.intentIdeaId);
						history.pushState(null, "", "?" + params.toString());
						setDraftId(resp.intentIdeaId);
					}
				})
				.catch((err) => {
					console.error("error getting daily status", err);
					if (isCurrent) setDailyStatusError(true);
				});
			return () => {
				isCurrent = false;
			};
		}
	}, [isDailyIntent, statusAttempt]);

	return (
		<AppWrapper>
			<AiSetupNeeded />
			{dailyStatusError && (
				<div
					role="alert"
					className="flex items-center justify-between gap-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 m-4 text-sm"
				>
					<span>
						Couldn&rsquo;t load today&rsquo;s daily intent status — a new log may not be
						linked to this session. You can keep talking, or try again.
					</span>
					<button
						className="underline font-semibold whitespace-nowrap"
						onClick={() => {
							setDailyStatusError(false);
							setStatusAttempt((attempt) => attempt + 1);
						}}
					>
						Try again
					</button>
				</div>
			)}
			{isLogModalOpen && (
				<DailyIntentModal setLogId={setLogId} onClose={() => setIsLogModalOpen(false)} />
			)}
			{/* key field so that rerender happens if daily intent draft idea found */}
			<ChatSection
				key={draftId}
				dailyLogId={logId}
				draftId={draftId ?? undefined}
				chatType={chatType}
				parentIdParam={parentId}
				fromGuideParam={isFromGuide}
				conversationEndCallbacks={() => {
					// end-of-session survey intentionally not part of the desktop app
				}}
			/>
		</AppWrapper>
	);
}
