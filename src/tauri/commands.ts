/**
 * The single IPC contract. Command names, event names, query-param
 * names and storage keys live here so a rename on either side of the
 * bridge is a one-file change caught by the compiler (and the
 * frontend/rust contract test).
 *
 * Type-only imports below: this module must stay free of runtime
 * dependencies (pages/index.astro reads STORAGE_KEYS at build time).
 */

import type { Channel } from "@tauri-apps/api/core";
import type { ChatMessage } from "@src/types";
import type { GenerationResult } from "@helpers/chat";
import type { RawFeedbackChild, RawIdea } from "@helpers/api/idea";
import type { RawDailyStatus, RawIdeaItem, RawUser } from "@helpers/api/user";
import type { DailyLogQuestion, LogAnswerItem } from "@helpers/api/forms";
import type { RawUserSettings } from "@helpers/api/settings";
import type {
	AiSettingsResponse,
	AppleSttStatus,
	DownloadEvent,
	EngineStatus,
	ModelsResponse
} from "@helpers/api/models";
import type { ExportIdeaResult, ImportShareResult } from "@helpers/api/share";
import type { IdeaReactions } from "@helpers/api/reactions";
import type { ChatType } from "@src/const";

export const COMMANDS = {
	getUser: "get_user",
	getDailyStatus: "get_daily_status",
	getAllIdeas: "get_all_ideas",
	getIdea: "get_idea",
	getIdeaChildren: "get_idea_children",
	createIdea: "create_idea",
	updateIdea: "update_idea",
	linkIdeaLog: "link_idea_log",
	setIdeaResult: "set_idea_result",
	getInputDevice: "get_input_device",
	listInputDevices: "list_input_devices",
	setInputDevice: "set_input_device",
	testInputDevice: "test_input_device",
	regenerateResult: "regenerate_result",
	openDataFolder: "open_data_folder",
	openLogFolder: "open_log_folder",
	copyDiagnostics: "copy_diagnostics",
	markIdeaRead: "mark_idea_read",
	deleteIdea: "delete_idea",
	addIdeaNote: "add_idea_note",
	updateIdeaNote: "update_idea_note",
	deleteIdeaNote: "delete_idea_note",
	getLogQuestions: "get_log_questions",
	submitLog: "submit_log",
	getSurveyFields: "get_survey_fields",
	submitSurvey: "submit_survey",
	getNotifications: "get_notifications",
	transcribe: "transcribe",
	/** one-shot generation: kept for tests, the UI streams via generateStreamingResponse */
	generateResponse: "generate_response",
	generateStreamingResponse: "generate_streaming_response",
	cancelGeneration: "cancel_generation",
	cancelTranscription: "cancel_transcription",
	sendTestNotification: "send_test_notification",
	startVoiceCapture: "start_voice_capture",
	stopVoiceCapture: "stop_voice_capture",
	stopAndTranscribe: "stop_and_transcribe",
	getUserSettings: "get_user_settings",
	saveUserSettings: "save_user_settings",
	setAppPresence: "set_app_presence",
	setQuickCaptureEnabled: "set_quick_capture_enabled",
	getUpdatesEnabled: "get_updates_enabled",
	setUpdatesEnabled: "set_updates_enabled",
	getAiSettings: "get_ai_settings",
	saveAiSettings: "save_ai_settings",
	testLlmEndpoint: "test_llm_endpoint",
	testSttEndpoint: "test_stt_endpoint",
	listModels: "list_models",
	getRuntimeStatus: "get_runtime_status",
	getAppleSttStatus: "get_apple_stt_status",
	getFreeDiskSpace: "get_free_disk_space",
	downloadModel: "download_model",
	cancelModelLoad: "cancel_model_load",
	cancelDownload: "cancel_download",
	pauseDownload: "pause_download",
	deleteModel: "delete_model",
	activateModel: "activate_model",
	exportIdea: "export_idea",
	exportMarkdown: "export_markdown",
	importShare: "import_share",
	getReactions: "get_reactions",
	toggleSectionReaction: "toggle_section_reaction",
	toggleCommentReaction: "toggle_comment_reaction"
} as const;

export type CommandName = (typeof COMMANDS)[keyof typeof COMMANDS];

/** Args shared by both generation commands (see helpers/api/ai.ts). */
type GenerateArgs = {
	messages: ChatMessage[];
	reactTo: string | null;
	reactToAuthor: string | null;
	reactToIsCurrentUser: boolean;
	chatType: ChatType | null;
};

type IdeaIdArgs = { ideaId: string };
type ModelIdArgs = { modelId: string };

/**
 * Per-command signatures: `args` is what the frontend sends (undefined =
 * no arguments), `result` what the Rust command resolves with. Use
 * `invokeCommand` (tauri/invoke.ts) so both are checked.
 */
interface CommandSignatures {
	getUser: { args: undefined; result: RawUser };
	getDailyStatus: { args: undefined; result: RawDailyStatus };
	getAllIdeas: { args: undefined; result: { ideas: RawIdeaItem[] } };
	getIdea: { args: IdeaIdArgs; result: RawIdea };
	getIdeaChildren: { args: IdeaIdArgs; result: { ideas: RawFeedbackChild[] } };
	createIdea: {
		args: {
			result: string;
			transcript: ChatMessage[];
			parentIdeaId: string | null;
			ideaMetadata: Record<string, unknown>;
			ideaType: ChatType;
			logId: string | null;
		};
		result: { id: string };
	};
	updateIdea: {
		args: {
			id: string;
			transcript?: ChatMessage[];
			result?: string;
			structuredResult?: unknown;
			title?: string;
		};
		result: { id: string };
	};
	markIdeaRead: { args: IdeaIdArgs; result: unknown };
	linkIdeaLog: { args: { ideaId: string; logId: string }; result: null };
	setIdeaResult: { args: { ideaId: string; result: string }; result: null };
	getInputDevice: { args: undefined; result: { name: string } };
	listInputDevices: { args: undefined; result: { name: string; isDefault: boolean }[] };
	setInputDevice: { args: { name?: string }; result: null };
	testInputDevice: { args: undefined; result: { peak: number } };
	regenerateResult: {
		args: { ideaId: string; onEvent: Channel };
		result: GenerationResult;
	};
	openDataFolder: { args: undefined; result: null };
	openLogFolder: { args: undefined; result: null };
	copyDiagnostics: { args: undefined; result: string };
	deleteIdea: { args: IdeaIdArgs; result: unknown };
	addIdeaNote: { args: { ideaId: string; note: string }; result: { id: string } };
	updateIdeaNote: {
		args: { ideaId: string; noteId: string; note: string };
		result: { id: string; noteId: string };
	};
	deleteIdeaNote: {
		args: { ideaId: string; noteId: string };
		result: { id: string; noteId: string };
	};
	getLogQuestions: { args: undefined; result: { log: DailyLogQuestion[] } };
	submitLog: { args: { log: LogAnswerItem[] }; result: { id: string } };
	// registered on the Rust side but not called by the frontend
	getSurveyFields: { args: undefined; result: unknown };
	submitSurvey: { args: { survey: unknown; ideaId?: string | null }; result: unknown };
	getNotifications: { args: undefined; result: { notifications: unknown[] } };
	sendTestNotification: { args: undefined; result: void };
	// raw WAV bytes, not an args object
	transcribe: { args: Uint8Array; result: { transcript: string } };
	generateResponse: { args: GenerateArgs; result: GenerationResult };
	generateStreamingResponse: {
		args: GenerateArgs & { summarize: boolean; onEvent: Channel };
		result: GenerationResult;
	};
	cancelGeneration: { args: undefined; result: void };
	cancelTranscription: { args: undefined; result: void };
	startVoiceCapture: { args: undefined; result: void };
	stopVoiceCapture: { args: undefined; result: ArrayBuffer };
	stopAndTranscribe: { args: undefined; result: { transcript: string } };
	getUserSettings: { args: undefined; result: RawUserSettings };
	saveUserSettings: { args: Record<string, unknown>; result: { id: string } };
	setAppPresence: { args: { dock: boolean; tray: boolean }; result: void };
	setQuickCaptureEnabled: { args: { enabled: boolean }; result: void };
	getUpdatesEnabled: { args: undefined; result: boolean };
	setUpdatesEnabled: { args: { enabled: boolean }; result: void };
	getAiSettings: { args: undefined; result: AiSettingsResponse };
	saveAiSettings: { args: { ai: object }; result: void };
	testLlmEndpoint: { args: { apiKey?: string } | undefined; result: string };
	testSttEndpoint: { args: { apiKey?: string } | undefined; result: string };
	listModels: { args: undefined; result: ModelsResponse };
	getRuntimeStatus: { args: undefined; result: { llm: EngineStatus; stt: EngineStatus } };
	getAppleSttStatus: { args: undefined; result: AppleSttStatus };
	getFreeDiskSpace: { args: undefined; result: number };
	downloadModel: { args: ModelIdArgs & { onEvent: Channel<DownloadEvent> }; result: void };
	cancelModelLoad: { args: undefined; result: null };
	cancelDownload: { args: ModelIdArgs; result: void };
	pauseDownload: { args: ModelIdArgs; result: void };
	deleteModel: { args: ModelIdArgs; result: void };
	activateModel: { args: ModelIdArgs; result: void };
	exportIdea: { args: IdeaIdArgs; result: ExportIdeaResult };
	exportMarkdown: {
		args: { ideaId: string; includeTranscript?: boolean };
		result: ExportIdeaResult;
	};
	importShare: { args: undefined; result: ImportShareResult };
	getReactions: { args: IdeaIdArgs; result: IdeaReactions };
	toggleSectionReaction: {
		args: { ideaId: string; sectionIndex: number; emoji: string };
		result: boolean;
	};
	toggleCommentReaction: {
		args: { feedbackIdeaId: string; itemIndex: number; emoji: string };
		result: boolean;
	};
}

/** Every command in COMMANDS with its args and result types (a command
 * missing from CommandSignatures is a compile error here). */
export type CommandMap = {
	[K in keyof typeof COMMANDS]: CommandSignatures[K];
};

/** Backend -> frontend events. */
export const EVENTS = {
	llmStatus: "llm-status",
	sttStatus: "stt-status",
	modelDownload: "model-download"
} as const;

/** Query params the app reads from window.location. */
export const QUERY_PARAMS = {
	id: "id",
	topic: "topic",
	qotd: "qotd",
	tab: "tab",
	parentId: "parentId",
	dailyIntent: "dailyIntent",
	/** where to go back to after a detour (same-app path only) */
	return: "return"
} as const;

export type QueryParam = (typeof QUERY_PARAMS)[keyof typeof QUERY_PARAMS];

/** localStorage keys (storage.ts and the index.astro inline script). */
export const STORAGE_KEYS = {
	gettingStartedDone: "has_done_getting_started",
	indexSeen: "has_seen_index",
	lastUpdateCheckMs: "last_update_check_ms",
	/** the recorder's input mode: "voice" | "text" */
	inputMode: "inputMode",
	/** the library's sort order (LibrarySort) */
	librarySort: "librarySort",
	/** the library's filter chips, comma-separated (LibraryFilter) */
	librarySortFilters: "librarySortFilters"
} as const;
