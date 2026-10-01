/**
 * The single IPC contract. Command names, event names, query-param
 * names and storage keys live here so a rename on either side of the
 * bridge is a one-file change caught by the compiler (and the
 * frontend/rust contract test).
 */

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
	isFeedbackAndFrom: "isFeedbackAndFrom"
} as const;

/** localStorage keys (storage.ts and the index.astro inline script). */
export const STORAGE_KEYS = {
	gettingStartedDone: "has_done_getting_started",
	indexSeen: "has_seen_index",
	lastUpdateCheckMs: "last_update_check_ms"
} as const;
