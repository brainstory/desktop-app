export interface Topic {
	topic: string;
	prompt: string;
	iconName: string;
}

export const TOPICS: Topic[] = [
	{
		topic: "Find motivation to start something",
		prompt: "You're on the right track! Tell me about what you've been meaning to start.",
		iconName: "rocket-outline"
	},
	{
		topic: "Outline a project or meeting",
		prompt: "That sounds like a great plan. What's the main goal or purpose?",
		iconName: "document-text-outline"
	},
	{
		topic: "Get more out of your day",
		prompt: "How has your day gone so far? What are you hoping to do with the rest of your day?",
		iconName: "sunny-outline"
	},
	{
		topic: "Develop a new habit or change an old one",
		prompt: "That's a great initiative! Do you have a new habit you're trying to form? Or a bad one you're trying to break?",
		iconName: "construct-outline"
	},
	{
		topic: "Talk about something new or interesting you learned",
		prompt: "Sounds interesting! Share what you learned. Did you enjoy the learning process?",
		iconName: "library-outline"
	},
	{
		topic: "Think through something that's been on your mind",
		prompt: "Of course, I'm here to help. What's on your mind?",
		iconName: "bulb-outline"
	}
];

/** The chat session's state machine (useChatSession). The values are
 * also what the UI compares against, so keep them stable. */
export const CONVERSATION_STATE = {
	Start: "starting new",
	Idle: "waiting for next user action (record, send, finish)",
	TranscribingUser: "transcribing...",
	ReadyToSendUserTranscript: "ready to send user message",
	WaitingForCoach: "sending message...",
	FinishWithResult: "finalizing..."
} as const;
export type ConversationState = (typeof CONVERSATION_STATE)[keyof typeof CONVERSATION_STATE];

/** Autosave indicator states (the values are the labels shown). */
export const CHAT_SAVE_STATE = {
	WAITING: "Waiting",
	SAVING: "Saving...",
	SUCCESS: "Saved",
	FAILED: "Autosave failed"
} as const;
export type ChatSaveState = (typeof CHAT_SAVE_STATE)[keyof typeof CHAT_SAVE_STATE];

export const MIN_CONVERSATION_LENGTH_BEFORE_SAVE: Record<string, number> = {
	DEFAULT: 4, // 4 messages = [assistant, user, assistant, user]
	daily_intent: 2 // 2 messages = [assistant, user]
};

export const CHAT_TYPE = {
	ORIGINAL: "original",
	FEEDBACK: "feedback",
	DAILY_INTENT: "daily_intent"
} as const;
export type ChatType = (typeof CHAT_TYPE)[keyof typeof CHAT_TYPE];

/** Sentinel user message recorded when the user asks for a different
 * question. Transcripts must not display it as a real answer. */
export const ASK_A_DIFFERENT_QUESTION = "Ask me a different question!";
