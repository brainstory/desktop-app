import { invoke } from "@tauri-apps/api/core";

export async function getDailyLogQuestionsApi() {
	const response = await invoke("get_log_questions");

	return response.log.map((question) => ({
		id: question.id,
		text: question.text,
		label: question.label
	}));
}

/**
 * Submit user's answers to their daily log
 * @param {list} logItems list of objects with field "id" and "value" [bool]
 * @returns
 */
export async function submitDailyLogQuestionsApi(logItems) {
	const response = await invoke("submit_log", { log: logItems });
	return response.id;
}

export async function getSurveyFieldsApi() {
	const response = await invoke("get_survey_fields");
	return { labels: response.ids, range: response.range };
}

/**
 * Submit user's survey response after completing an idea
 * @param {list} surveyItems list of objects with field "id" and "value" [int]
 * @param {int} ideaId idea the survey is in resposne to
 * @returns
 */
export async function submitSurveyResponseApi(surveyItems, ideaId) {
	const response = await invoke("submit_survey", { survey: surveyItems, ideaId });
	return response.id;
}

export default {
	getDailyLogQuestionsApi,
	submitDailyLogQuestionsApi,
	getSurveyFieldsApi,
	submitSurveyResponseApi
};
