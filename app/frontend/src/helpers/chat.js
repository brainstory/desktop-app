import { useEffect } from "react";
import { TOPICS, CHAT_TYPE } from "@src/const";
import { getQuestionOfTheDay } from "@helpers/qotd";
import { getQueryParam } from "@helpers/helpers";
import { getIdeaApi } from "@helpers/api/idea";

export function useIdeaIdFromUrl(hasMounted, setIdeaId) {
	useEffect(() => {
		if (hasMounted.current) {
			return;
		}
		// this is for safeguarding against extra idea saves from weird edges cases
		// related to the ideaId once being in the query parameter but isn't anymore
		setIdeaId(getQueryParam("id"));
		hasMounted.current = true;
	}, []);
}

export function getFirstPrompt(chatType) {
	const isQotd = getQueryParam("qotd") === null ? false : true;
	const isTopicStarter = getQueryParam("topic");

	let firstPrompt = "Hi, how's it going? What's on your mind?";
	if (chatType === CHAT_TYPE.FEEDBACK) {
		firstPrompt = `Would you like to extend this idea, react to specific points, or walk through the idea with me point by point?`;
	} else if (chatType === CHAT_TYPE.DAILY_INTENT) {
		firstPrompt = "Walk me through how you want your day to go.";
	} else if (isQotd) {
		firstPrompt = getQuestionOfTheDay();
	} else if (isTopicStarter) {
		firstPrompt = TOPICS[isTopicStarter].prompt;
	}

	return firstPrompt;
}

export const addConversationMessage = (
	message,
	isUser,
	conversation,
	setConversation,
	callback
) => {
	const role = isUser ? "user" : "assistant";
	let updateCurrConversation = [
		...conversation,
		{
			role: role,
			content: message
		}
	];
	setConversation(updateCurrConversation);
	callback();
};

export const removeLastConversationMessage = (conversation, setConversation) => {
	const removedMessage = conversation.pop();
	setConversation([...conversation]);
	return removedMessage.content;
};

export const handleWebsocketStreamResult = async (
	generateResultStream,
	updateMessage,
	successCallbacks,
	onError
) => {
	const { channel, invokePromise } = generateResultStream();
	let message = "";
	channel.onmessage = (event) => {
		if (event.type === "chunk") {
			message = message + event.content;
			updateMessage(message);
		}
		if (event.type === "cumulative") {
			message = event.content;
			updateMessage(message);
		}
	};
	try {
		const result = await invokePromise;
		if (result?.response) {
			message = result.response;
			updateMessage(message);
		}
		successCallbacks(message, result?.structured_result ?? null);
	} catch (err) {
		console.log("streaming result failed", err);
		if (onError) {
			onError(err);
		}
	}
};

export function findMostRecentAssistantContent(currConversation) {
	for (let i = currConversation.length - 1; i >= 0; i--) {
		const message = currConversation[i];
		if (message.role === "assistant") {
			return message.content;
		}
	}
	return null;
}

export function findMostRecentUserContent(currConversation) {
	for (let i = currConversation.length - 1; i >= 0; i--) {
		const message = currConversation[i];
		if (message.role === "user") {
			return message.content;
		}
	}
	return null;
}
