import { invoke } from "@tauri-apps/api/core";

export interface DailyLogQuestionApi {
	id: number;
	text: string;
	label: string;
}

export interface LogAnswerItem {
	id: number;
	value: boolean;
}

/** A daily-log question bound to its current answer (settings/log UIs). */
export interface LogFormAnswer {
	id: number;
	text?: string;
	value: boolean;
}

export async function getDailyLogQuestionsApi(): Promise<DailyLogQuestionApi[]> {
	const response = await invoke<{ log: DailyLogQuestionApi[] }>("get_log_questions");

	return response.log.map((question) => ({
		id: question.id,
		text: question.text,
		label: question.label
	}));
}

/**
 * Submit user's answers to their daily log
 * @param logItems list of objects with field "id" and "value" [bool]
 */
export async function submitDailyLogQuestionsApi(logItems: LogAnswerItem[]): Promise<string> {
	const response = await invoke<{ id: string }>("submit_log", { log: logItems });
	return response.id;
}

export default {
	getDailyLogQuestionsApi,
	submitDailyLogQuestionsApi
};
