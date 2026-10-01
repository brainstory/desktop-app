import { invoke } from "@tauri-apps/api/core";
import { COMMANDS } from "@src/tauri/commands";

export interface DailyLogQuestion {
	id: number;
	text: string;
	label: string;
}

export interface LogSettingsQuestion {
	id: number;
	label: string;
	questionText: string;
	enabled: boolean;
}

export interface NotificationSetting {
	title: string;
	description?: string | null;
	value?: string | null;
	valueType: string;
	enabled: boolean;
}

export interface UserSettingsResponse {
	user: {
		name?: string | null;
		timezone?: string | null;
	};
	dailyLog: {
		id: number;
		label: string;
		questionText: string;
		enabled: boolean;
	}[];
	notifications: {
		title: string;
		description?: string | null;
		value?: string | null;
		valueType: string;
		enabled: boolean;
	}[];
	presence: {
		dock: boolean;
		tray: boolean;
	};
	updates: {
		enabled: boolean;
	};
}

/** Get user settings */
export async function getUserSettingsApi(): Promise<UserSettingsResponse> {
	const response = await invoke<{
		user: { name?: string; timezone?: string };
		log: { id: number; label: string; text: string; enabled: boolean }[];
		notifications: {
			title: string;
			description?: string;
			value?: string;
			value_type: string;
			enabled: boolean;
		}[];
		presence: { dock: boolean; tray: boolean };
		updates?: { enabled: boolean };
	}>(COMMANDS.getUserSettings);

	const user = {
		name: response.user?.name,
		timezone: "timezone" in response.user ? response.user.timezone : ""
	};

	const dailyLogQuestions = response.log.map((field) => ({
		id: field.id,
		label: field.label,
		questionText: field.text,
		enabled: field.enabled
	}));

	const notifications = response.notifications.map((field) => ({
		title: field.title,
		description: field?.description,
		value: field.value,
		valueType: field.value_type,
		enabled: field.enabled
	}));

	return {
		user: user,
		dailyLog: dailyLogQuestions,
		notifications: notifications,
		presence: {
			dock: response.presence?.dock ?? true,
			tray: response.presence?.tray ?? true
		},
		updates: {
			enabled: response.updates?.enabled ?? true
		}
	};
}

/** Toggle dock / menu-bar (tray) icon visibility */
export function setAppPresenceApi(dock: boolean, tray: boolean): Promise<void> {
	return invoke(COMMANDS.setAppPresence, { dock, tray });
}

/** Whether automatic update checks are on (default true). */
export function getUpdatesEnabledApi(): Promise<boolean> {
	return invoke(COMMANDS.getUpdatesEnabled);
}

/** Opt in to / out of automatic update checks. */
export function setUpdatesEnabledApi(enabled: boolean): Promise<void> {
	return invoke(COMMANDS.setUpdatesEnabled, { enabled });
}

/** Save user settings */
export interface SaveUserSettingsOptions {
	name?: string | null;
	timezone?: string | null;
	enabledLogQids?: number[] | null;
	notifications?: Record<string, unknown>[] | null;
}

/** Persist user settings. Only the provided fields are sent. */
export async function saveUserSettingsApi({
	name,
	timezone,
	enabledLogQids,
	notifications
}: SaveUserSettingsOptions = {}): Promise<{ id: string }> {
	const body: Record<string, unknown> = {};
	if (name) {
		body.user = { name };
	}
	if (timezone) {
		body.user = { ...(body.user as object), timezone };
	}
	if (enabledLogQids) {
		body.enabled_log_question_ids = enabledLogQids;
	}
	if (notifications) {
		body.notifications = notifications;
	}
	const response = await invoke<{ id: string }>(COMMANDS.saveUserSettings, body);
	return response;
}
