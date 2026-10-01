import { invokeCommand } from "@src/tauri/invoke";

/** A daily-log question as the backend lists it. */
export interface DailyLogQuestion {
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

export async function getDailyLogQuestionsApi(): Promise<DailyLogQuestion[]> {
	const response = await invokeCommand("getLogQuestions");

	return response.log.map((question) => ({
		id: question.id,
		text: question.text,
		label: question.label
	}));
}

/**
 * Submit user's answers to their daily log
 * @param logItems list of objects with field COMMANDS.id and "value" [bool]
 */
export async function submitDailyLogQuestionsApi(logItems: LogAnswerItem[]): Promise<string> {
	const response = await invokeCommand("submitLog", { log: logItems });
	return response.id;
}

export default {
	getDailyLogQuestionsApi,
	submitDailyLogQuestionsApi
};
