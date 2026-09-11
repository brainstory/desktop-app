import { invoke } from "@tauri-apps/api/core";

/** Get user settings */
export async function getUserSettingsApi() {
	const response = await invoke("get_user_settings");

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
export async function setAppPresenceApi(dock, tray) {
	return invoke("set_app_presence", { dock, tray });
}

/** Save user settings */
export async function saveUserSettingsApi(
	newName = null,
	newTimezone = null,
	enabledLogQids = null,
	notifications = null
) {
	let body = {};
	if (newName) {
		body.user = {};
		body.user.name = newName;
	}
	if (newTimezone) {
		if (!("user" in body)) body.user = {};
		body.user.timezone = newTimezone;
	}
	if (enabledLogQids) {
		body.enabled_log_question_ids = enabledLogQids;
	}
	if (notifications) {
		body.notifications = notifications;
	}
	const response = await invoke("save_user_settings", body);
	return response.id;
}
