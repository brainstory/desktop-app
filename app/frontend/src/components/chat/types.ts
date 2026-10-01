/** Shapes shared across the chat components. */

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
