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

/**
 * Tauri commands reject with a plain string; everything else (fetch, JS
 * errors) rejects with an Error-like object. Normalize to a string so
 * callers can always inspect the message.
 */
export function normalizeApiError(error) {
	if (typeof error === "string") return error;
	return error?.message ?? String(error);
}

/**
 * The backend's moderation signal: external providers' content-filter
 * rejections are mapped by the Rust layer onto the original protocol's
 * "HttpError 469" marker. Detect it by prefix, not substring, so an error
 * that merely *quotes* the marker (e.g. in a wrapped message) can't
 * trigger the resend flow.
 */
export function isModerationError(message) {
	return normalizeApiError(message).startsWith(ERROR_MESSAGE_MAP[469]);
}

export async function callApiWithRetry(apiCall, retriesLeft = 1) {
	return new Promise((resolve, reject) => {
		apiCall()
			.then((message) => {
				resolve(message);
			})
			.catch((error) => {
				if (isModerationError(error)) {
					reject(error);
					return;
				}
				if (retriesLeft >= 1) {
					setTimeout(async () => {
						retriesLeft--;
						try {
							const result = await callApiWithRetry(apiCall, retriesLeft);
							resolve(result);
						} catch (retryError) {
							reject(retryError);
						}
					}, 500);
				} else {
					reject(error);
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
}
