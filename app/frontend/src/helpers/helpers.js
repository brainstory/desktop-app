import md5 from "md5";
import { ERROR_MESSAGE_MAP } from "@src/const";

export function getQueryParam(name) {
	const urlParams = new URLSearchParams(window.location.search);
	return urlParams.get(name);
}

export function formatISO8601ToHumanReadable(
	iso8601Date,
	options = {
		year: "numeric",
		month: "short",
		day: "numeric",
		hour: "numeric",
		minute: "2-digit"
	}
) {
	// Add "Z" so it converts to users local time zone
	const date = new Date(iso8601Date + "Z");
	return date.toLocaleDateString("en-US", options);
}

export function getTimeDifferenceFromToday(trialEndAt) {
	const dateEnd = new Date(trialEndAt);
	const today = new Date();
	const todayUtc = Date.UTC(
		today.getUTCFullYear(),
		today.getUTCMonth(),
		today.getUTCDate(),
		today.getUTCHours(),
		today.getUTCMinutes(),
		today.getUTCSeconds()
	);

	return dateEnd.valueOf() - todayUtc.valueOf();
}

export async function callApiWithRetry(apiCall, retriesLeft = 1) {
	return new Promise((resolve, reject) => {
		apiCall()
			.then((message) => {
				resolve(message);
			})
			.catch((error) => {
				if (error.message.includes(ERROR_MESSAGE_MAP[469])) {
					console.log("THROW ERROR AGAIN INSTEAD OF RETRY");
					reject(error);
					throw error;
				}
				if (retriesLeft >= 1) {
					setTimeout(async () => {
						console.log("Retrying after error", error.message);
						retriesLeft--;
						try {
							const result = await callApiWithRetry(apiCall, retriesLeft);
							resolve(result);
						} catch (retryError) {
							reject(retryError); // Reject the promise if retries are exhausted
						}
					}, 500);
				} else {
					console.log("No api retries left", error);
					reject(error); // Reject the promise if retries are exhausted
				}
			});
	});
}

/**
 * Local avatar placeholder (no external requests in the desktop app).
 * Renders the first letter of the name/email on a pink disc.
 */
export function getGravatarUrl(emailOrName) {
	const letter = (emailOrName || "?").trim().charAt(0).toUpperCase() || "?";
	const svg = `<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64'><rect width='64' height='64' rx='32' fill='#fce7f3'/><text x='32' y='42' font-family='sans-serif' font-size='28' font-weight='600' fill='#db2777' text-anchor='middle'>${letter}</text></svg>`;
	return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
};

/** Email validation with regex */
export function isEmailValid(email) {
	const regex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
	return regex.test(email);
}
