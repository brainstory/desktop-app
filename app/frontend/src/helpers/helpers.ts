import { ERROR_MESSAGE_MAP } from "@src/const";

export function getQueryParam(name: string): string | null {
	const urlParams = new URLSearchParams(window.location.search);
	return urlParams.get(name);
}

/**
 * Parse the backend's naive-UTC timestamps ("2026-09-15T10:30:00") as
 * UTC (a "Z" is appended unless the string already carries a zone).
 * Null for anything empty or unparseable - callers render a placeholder
 * instead of "Invalid Date".
 */
export function parseBackendUtc(iso8601Date?: string | null): Date | null {
	if (!iso8601Date) return null;
	const trimmed = iso8601Date.trim();
	if (!trimmed) return null;
	const hasZone = /(?:Z|[+-]\d{2}:?\d{2})$/i.test(trimmed);
	const date = new Date(hasZone ? trimmed : `${trimmed}Z`);
	return Number.isNaN(date.getTime()) ? null : date;
}

export function formatISO8601ToHumanReadable(
	iso8601Date: string,
	options: Intl.DateTimeFormatOptions = {
		year: "numeric",
		month: "short",
		day: "numeric",
		hour: "numeric",
		minute: "2-digit"
	}
): string {
	const date = parseBackendUtc(iso8601Date);
	if (!date) {
		// empty/garbage input (e.g. a null createdAt fallback) must render
		// a placeholder, not "Invalid Date"
		return "—";
	}
	return date.toLocaleDateString("en-US", options);
}

/**
 * Tauri commands reject with a plain string; everything else (fetch, JS
 * errors) rejects with an Error-like object. Normalize to a string so
 * callers can always inspect the message.
 */
export function normalizeApiError(error: unknown): string {
	if (typeof error === "string") return error;
	if (error instanceof Error) return error.message;
	return String(error);
}

/**
 * The backend's moderation signal: external providers' content-filter
 * rejections are mapped by the Rust layer onto the original protocol's
 * "HttpError 469" marker. Detect it by prefix, not substring, so an error
 * that merely *quotes* the marker (e.g. in a wrapped message) can't
 * trigger the resend flow.
 */
export function isModerationError(message: unknown): boolean {
	return normalizeApiError(message).startsWith(ERROR_MESSAGE_MAP[469]);
}

export async function callApiWithRetry<T>(
	apiCall: () => Promise<T>,
	retriesLeft = 1,
	/** Decide whether an error is worth retrying; defaults to always
	 * (except moderation errors, which never retry). */
	shouldRetry: (error: unknown) => boolean = () => true
): Promise<T> {
	return new Promise<T>((resolve, reject) => {
		apiCall()
			.then((message) => {
				resolve(message);
			})
			.catch((error: unknown) => {
				if (isModerationError(error) || !shouldRetry(error)) {
					reject(error);
					return;
				}
				if (retriesLeft >= 1) {
					setTimeout(() => {
						callApiWithRetry(apiCall, retriesLeft - 1, shouldRetry).then(
							resolve,
							reject
						);
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
export function getGravatarUrl(emailOrName?: string | null): string {
	const letter = (emailOrName || "?").trim().charAt(0).toUpperCase() || "?";
	const svg = `<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64'><rect width='64' height='64' rx='32' fill='#fce7f3'/><text x='32' y='42' font-family='sans-serif' font-size='28' font-weight='600' fill='#db2777' text-anchor='middle'>${letter}</text></svg>`;
	return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}
