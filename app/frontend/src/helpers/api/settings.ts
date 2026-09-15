import { invoke } from "@tauri-apps/api/core";

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
	}>("get_user_settings");

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
		}
	};
}

/** Toggle dock / menu-bar (tray) icon visibility */
export function setAppPresenceApi(dock: boolean, tray: boolean): Promise<void> {
	return invoke("set_app_presence", { dock, tray });
}

/** Save user settings */
export async function saveUserSettingsApi(
	newName: string | null = null,
	newTimezone: string | null = null,
	enabledLogQids: number[] | null = null,
	notifications: Record<string, unknown>[] | null = null
): Promise<{ id: string }> {
	const body: Record<string, unknown> = {};
	if (newName) {
		body.user = { name: newName };
	}
	if (newTimezone) {
		body.user = { ...(body.user as object), timezone: newTimezone };
	}
	if (enabledLogQids) {
		body.enabled_log_question_ids = enabledLogQids;
	}
	if (notifications) {
		body.notifications = notifications;
	}
	const response = await invoke<{ id: string }>("save_user_settings", body);
	return response;
}
