import { useState, useEffect } from "react";

import { getUserDailyStatusApi } from "@helpers/api/user";
import { getQueryParam } from "@helpers/helpers";
import { CHAT_TYPE } from "@src/const";

import { ChatSection } from "./ChatSection";
import { AppWrapper } from "@components/chat/reusable/AppWrapper";
import DailyIntentModal from "@components/form/DailyIntentModal";
import AiSetupNeeded from "@components/dashboard/AiSetupNeeded";

/** Resolve the chat mode from the URL once, inside the component (module
 * scope would read window.location at import time and make the module
 * untestable). */
function chatTypeFromParams(): string {
	const parentId = getQueryParam("parentId");
	if (getQueryParam("dailyIntent") === "true") {
		return CHAT_TYPE.DAILY_INTENT;
	}
	if (parentId) {
		return CHAT_TYPE.FEEDBACK;
	}
	return CHAT_TYPE.ORIGINAL;
}

export default function ChatApp() {
	const isDailyIntent = getQueryParam("dailyIntent") === "true";
	const chatType = chatTypeFromParams();
	const parentId = getQueryParam("parentId");
	const isFromGuide = getQueryParam("topic");
	const [logId, setLogId] = useState<string | null | undefined>(null);
	const [draftId, setDraftId] = useState(getQueryParam("id"));
	const [isLogModalOpen, setIsLogModalOpen] = useState(false);

	useEffect(() => {
		if (isDailyIntent) {
			getUserDailyStatusApi()
				.then((resp) => {
					if (!resp.logId) {
						setIsLogModalOpen(true);
					}
					setLogId(resp.logId);
					if (resp.intentIdeaId) {
						// intent idea draft is found
						const url = new URL(window.location.href);
						const params = new URLSearchParams(url.search);
						params.set("id", resp.intentIdeaId);
						history.pushState(null, "", "?" + params.toString());
						setDraftId(resp.intentIdeaId);
					}
				})
				.catch((err) => console.error("error getting daily status", err));
		}
	}, [isDailyIntent]);

	return (
		<AppWrapper>
			<AiSetupNeeded />
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
