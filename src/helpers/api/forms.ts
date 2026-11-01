import { invokeCommand } from "@src/tauri/invoke";
import type { LogAnswerItem as RawLogAnswerItem } from "@src/tauri/generated/LogAnswerItem";
import type { LogQuestionItem } from "@src/tauri/generated/LogQuestionItem";

/** A daily-log question as the backend lists it (today's answer, when
 * one is saved, rides along in `value`). */
export type DailyLogQuestion = LogQuestionItem;

export type LogAnswerItem = RawLogAnswerItem;

/** A daily-log question bound to its current answer (settings/log UIs). */
export interface LogFormAnswer {
	id: number;
	text?: string;
	value: boolean;
}

export async function getDailyLogQuestionsApi(): Promise<DailyLogQuestion[]> {
	const response: { log: LogQuestionItem[] } = await invokeCommand("getLogQuestions");

	return response.log.map((question) => ({
		id: question.id,
		text: question.text,
		label: question.label,
		// pass today's saved answer through; false only when absent
		value: question.value
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
