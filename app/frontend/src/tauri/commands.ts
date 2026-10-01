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
import type { DailyLogQuestionApi, LogAnswerItem } from "@helpers/api/forms";
import type { RawUserSettings } from "@helpers/api/settings";
import type {
	AiSettingsResponse,
	AppleSttStatus,
	DownloadEvent,
	EngineStatus,
	ModelsResponse
} from "@helpers/api/models";
import type { ExportIdeaResult, ImportShareResult } from "@helpers/api/share";

export const COMMANDS = {
	getUser: "get_user",
	getDailyStatus: "get_daily_status",
	getAllIdeas: "get_all_ideas",
	getIdea: "get_idea",
	getIdeaChildren: "get_idea_children",
	createIdea: "create_idea",
	updateIdea: "update_idea",
	markIdeaRead: "mark_idea_read",
	deleteIdea: "delete_idea",
	getLogQuestions: "get_log_questions",
	submitLog: "submit_log",
	getSurveyFields: "get_survey_fields",
	submitSurvey: "submit_survey",
	getNotifications: "get_notifications",
	transcribe: "transcribe",
	generateResponse: "generate_response",
	generateStreamingResponse: "generate_streaming_response",
	cancelGeneration: "cancel_generation",
	sendTestNotification: "send_test_notification",
	startVoiceCapture: "start_voice_capture",
	stopVoiceCapture: "stop_voice_capture",
	getUserSettings: "get_user_settings",
	saveUserSettings: "save_user_settings",
	setAppPresence: "set_app_presence",
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
	cancelDownload: "cancel_download",
	deleteModel: "delete_model",
	activateModel: "activate_model",
	exportIdea: "export_idea",
	importShare: "import_share"
} as const;

export type CommandName = (typeof COMMANDS)[keyof typeof COMMANDS];

/** Args shared by both generation commands (see helpers/api/ai.ts). */
type GenerateArgs = {
	messages: ChatMessage[];
	reactTo: string | null;
	reactToAuthor: string | null;
	reactToIsCurrentUser: boolean;
	chatType: string | null;
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
			ideaType: string;
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
	deleteIdea: { args: IdeaIdArgs; result: unknown };
	getLogQuestions: { args: undefined; result: { log: DailyLogQuestionApi[] } };
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
	startVoiceCapture: { args: undefined; result: void };
	stopVoiceCapture: { args: undefined; result: ArrayBuffer };
	getUserSettings: { args: undefined; result: RawUserSettings };
	saveUserSettings: { args: Record<string, unknown>; result: { id: string } };
	setAppPresence: { args: { dock: boolean; tray: boolean }; result: void };
	getUpdatesEnabled: { args: undefined; result: boolean };
	setUpdatesEnabled: { args: { enabled: boolean }; result: void };
	getAiSettings: { args: undefined; result: AiSettingsResponse };
	saveAiSettings: { args: { ai: object }; result: void };
	testLlmEndpoint: { args: undefined; result: string };
	testSttEndpoint: { args: undefined; result: string };
	listModels: { args: undefined; result: ModelsResponse };
	getRuntimeStatus: { args: undefined; result: { llm: EngineStatus; stt: EngineStatus } };
	getAppleSttStatus: { args: undefined; result: AppleSttStatus };
	getFreeDiskSpace: { args: undefined; result: number };
	downloadModel: { args: ModelIdArgs & { onEvent: Channel<DownloadEvent> }; result: void };
	cancelDownload: { args: ModelIdArgs; result: void };
	deleteModel: { args: ModelIdArgs; result: void };
	activateModel: { args: ModelIdArgs; result: void };
	exportIdea: { args: IdeaIdArgs; result: ExportIdeaResult };
	importShare: { args: undefined; result: ImportShareResult };
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
	dailyIntent: "dailyIntent"
} as const;

/** localStorage keys (storage.ts and the index.astro inline script). */
export const STORAGE_KEYS = {
	gettingStartedDone: "has_done_getting_started",
	indexSeen: "has_seen_index",
	lastUpdateCheckMs: "last_update_check_ms"
} as const;
