/** Shapes shared across the chat components. */

/** Where a chat error came from: generating a reply, or saving the
 * session. Decides the banner's heading and whether it links to the AI
 * settings. */
export type ChatErrorSource = "ai" | "save";

/** The idea a feedback chat reacts to. */
export interface ParentIdea {
	id: string;
	title?: string | null;
	/** the idea's result document (what the coach reacts to) */
	summary?: string | null;
	/** set only for ideas imported from someone else; null/undefined
	 * means the user wrote it */
	creatorName?: string | null;
}
