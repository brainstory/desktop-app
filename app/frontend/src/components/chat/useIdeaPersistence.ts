/**
 * Idea persistence for the chat: create-once, debounced sequenced
 * autosave, draft loading, and parent-idea fetching. Extracted from
 * ChatSection so the component handles rendering + generation flow.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type { ChatMessage } from "@src/types";
import { CHAT_SAVE_STATE } from "@src/const";
import { createIdeaApi, getIdeaApi, updateIdeaApi } from "@helpers/api/idea";
import { normalizeApiError } from "@helpers/helpers";

/** Load failures that replace the whole chat UI with an error section. */
export type ChatFatalError = "draft-not-found" | "parent-not-found";

export interface IdeaPersistenceOptions {
	chatType: string;
	parentIdParam: string | null;
	dailyLogId?: string | null;
	/** the draft id from the URL, seeding the initial ideaId */
	initialIdeaId?: string;
	/** minimum conversation length before an idea row is created */
	minLength: number;
	/** called for error surfacing in the parent */
	onError: (message: string) => void;
	/** called for fatal load failures (the parent renders the section) */
	onFatalError: (error: ChatFatalError) => void;
	/** fetch and display the parent idea (feedback flow) */
	onParentIdea: (parentId: string) => void;
}

export function useIdeaPersistence(
	conversation: ChatMessage[],
	result: string,
	options: IdeaPersistenceOptions
) {
	const {
		chatType,
		parentIdParam,
		dailyLogId,
		minLength,
		initialIdeaId,
		onError,
		onFatalError,
		onParentIdea
	} = options;

	const [ideaId, setIdeaId] = useState<string | undefined>(initialIdeaId);
	const [saveState, setSaveState] = useState(CHAT_SAVE_STATE.WAITING);

	// refs so the effects see live values without re-running
	const creatingIdeaRef = useRef(false);
	const conversationLengthRef = useRef(conversation.length);
	const fetchedParentRef = useRef<string | null>(null);
	const onErrorRef = useRef(onError);
	useEffect(() => {
		onErrorRef.current = onError;
	});

	// Autosave plumbing. Every write goes through one promise chain, so at
	// most one updateIdeaApi is in flight and writes land in order; the
	// pending slot coalesces bursts so the latest conversation wins.
	const pendingSaveRef = useRef<{ ideaId: string; conversation: ChatMessage[] } | null>(null);
	const saveQueueRef = useRef<Promise<unknown>>(Promise.resolve());
	const drainQueuedRef = useRef(false);
	const debounceRef = useRef<number | null>(null);
	/** the conversation already in the database (created, loaded or saved):
	 * autosaving it again would be a redundant write */
	const persistedRef = useRef<ChatMessage[] | null>(null);

	const markPersisted = useCallback((saved: ChatMessage[]) => {
		persistedRef.current = saved;
	}, []);

	const flushAutosave = useCallback(() => {
		if (debounceRef.current !== null) {
			window.clearTimeout(debounceRef.current);
			debounceRef.current = null;
		}
		// a queued (not yet started) step reads the pending slot when it
		// runs, so it already covers this flush
		if (!pendingSaveRef.current || drainQueuedRef.current) return;
		drainQueuedRef.current = true;
		saveQueueRef.current = saveQueueRef.current.then(async () => {
			drainQueuedRef.current = false;
			const job = pendingSaveRef.current;
			pendingSaveRef.current = null;
			if (!job) return;
			setSaveState(CHAT_SAVE_STATE.SAVING);
			try {
				await updateIdeaApi(job.ideaId, job.conversation);
				persistedRef.current = job.conversation;
				// a newer save is queued: leave the label to it
				if (pendingSaveRef.current) return;
				// don't show SAVED visual for saving the user message so
				// the switch from SAVING to SAVED doesn't happen twice
				if (job.conversation[job.conversation.length - 1]?.role === "assistant") {
					setSaveState(CHAT_SAVE_STATE.SUCCESS);
				}
			} catch (e) {
				if (pendingSaveRef.current) return;
				setSaveState(CHAT_SAVE_STATE.FAILED);
				onErrorRef.current(`Autosave failed: ${normalizeApiError(e)}`);
			}
		});
	}, []);

	/** Save the final result, ordered after any autosave already in flight
	 * (a late autosave would otherwise land after it). Rejects on failure. */
	const saveResult = useCallback(
		(
			id: string,
			finalConversation: ChatMessage[],
			resultText: string,
			structuredResult: unknown
		): Promise<void> => {
			if (debounceRef.current !== null) {
				window.clearTimeout(debounceRef.current);
				debounceRef.current = null;
			}
			// the final write carries the full conversation
			pendingSaveRef.current = null;
			const write = saveQueueRef.current.then(async () => {
				await updateIdeaApi(id, finalConversation, resultText, structuredResult);
				persistedRef.current = finalConversation;
			});
			saveQueueRef.current = write.catch(() => {});
			return write;
		},
		[]
	);

	useEffect(() => {
		conversationLengthRef.current = conversation.length;
	}, [conversation.length]);

	// derived: an idea must be created as soon as the conversation is long
	// enough and no idea row exists yet
	const readyToCreateIdea = !ideaId && conversation.length >= minLength;

	// create-once effect: the guard prevents a second create while one is
	// in flight (StrictMode double-invoke, conversation updates). The
	// conversation is a dependency so a failed create retries on the next
	// message instead of never saving the session.
	useEffect(() => {
		if (readyToCreateIdea && !creatingIdeaRef.current) {
			creatingIdeaRef.current = true;
			const createdWith = conversation;
			createIdeaApi(result, createdWith, chatType, parentIdParam, dailyLogId)
				.then((createdIdeaId) => {
					creatingIdeaRef.current = false;
					// the create already stored this conversation
					persistedRef.current = createdWith;
					setIdeaId(createdIdeaId);
					const url = new URL(window.location.href);
					const params = new URLSearchParams(url.search);
					params.set("id", createdIdeaId);
					history.pushState(null, "", "?" + params.toString());
				})
				.catch((err) => {
					creatingIdeaRef.current = false;
					onError(`Could not save this session: ${normalizeApiError(err)}`);
					// readyToCreateIdea stays true; the next conversation
					// update re-runs this effect and retries creation
				});
		}
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [readyToCreateIdea, dailyLogId, conversation]);

	// Autosave: debounced; the write itself is serialized by flushAutosave
	useEffect(() => {
		if (!ideaId || conversation.length < minLength) {
			return;
		}
		if (conversation === persistedRef.current) {
			return;
		}
		pendingSaveRef.current = { ideaId, conversation };
		if (debounceRef.current !== null) {
			window.clearTimeout(debounceRef.current);
		}
		debounceRef.current = window.setTimeout(flushAutosave, 400);
	}, [conversation, ideaId, minLength, flushAutosave]);

	// Never drop the last change: flush the pending save when the chat
	// unmounts or the page goes away (Astro navigations are full page
	// loads, so React never unmounts on them)
	useEffect(() => {
		window.addEventListener("pagehide", flushAutosave);
		return () => {
			window.removeEventListener("pagehide", flushAutosave);
			flushAutosave();
		};
	}, [flushAutosave]);

	return {
		ideaId,
		setIdeaId,
		saveState,
		setSaveState,
		readyToCreateIdea,
		conversationLengthRef,
		markPersisted,
		saveResult,
		getIdeaApi,
		updateIdeaApi,
		onFatalError,
		onParentIdea,
		fetchedParentRef
	};
}

/** Draft + parent idea loading, split from the autosave (different deps). */
export function useDraftLoader(
	ideaId: string | undefined,
	parentIdParam: string | null,
	hooks: {
		conversationLengthRef: React.RefObject<number>;
		fetchedParentRef: React.RefObject<string | null>;
		setIdeaId: (id: string | undefined) => void;
		setCurrConversation: (next: ChatMessage[]) => void;
		/** tell the autosave the loaded transcript is already stored */
		markPersisted: (saved: ChatMessage[]) => void;
		setConversationState: (state: string) => void;
		onParentIdea: (parentId: string) => void;
		onFatalError: (error: ChatFatalError) => void;
	}
) {
	const {
		conversationLengthRef,
		fetchedParentRef,
		setCurrConversation,
		markPersisted,
		setConversationState,
		onParentIdea,
		onFatalError
	} = hooks;

	useEffect(() => {
		if (ideaId) {
			getIdeaApi(ideaId)
				.then((res) => {
					if (res.summary) {
						// if result already present, change to idea result page
						window.location.href = `/idea?id=${res.id}`;
						return;
					}
					const savedConversation = [...(res.transcript ?? [])];
					// Only adopt the saved transcript if it has more messages
					// than what we hold locally (via the ref: this closure
					// sees the mount-time conversation): restores a resumed
					// draft, but never clobbers newer messages.
					if (savedConversation.length > conversationLengthRef.current) {
						markPersisted(savedConversation);
						setCurrConversation(savedConversation);
						const lastMessage = savedConversation.at(-1);
						if (lastMessage?.role === "user") {
							setConversationState("ready to send user message");
						} else {
							setConversationState(
								"waiting for next user action (record, send, finish)"
							);
						}
					}

					const parentIdData = res.parentIdea?.id;
					if (parentIdData && fetchedParentRef.current !== parentIdData) {
						fetchedParentRef.current = parentIdData;
						onParentIdea(parentIdData);
					}
				})
				.catch((err) => {
					console.error("idea not found with ID", ideaId, err);
					onFatalError("draft-not-found");
				});
		} else if (parentIdParam && fetchedParentRef.current !== parentIdParam) {
			fetchedParentRef.current = parentIdParam;
			onParentIdea(parentIdParam);
		}
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [ideaId, parentIdParam]);
}
