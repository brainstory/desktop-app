// Simple persisted flags for onboarding state. Cookies turned out to be
// unreliable in the Tauri webview (custom scheme + Secure cookies), so
// local storage is the durable mechanism in the desktop app.

import { STORAGE_KEYS } from "@src/tauri/commands";

const getFlag = (name: string): string | null => {
	try {
		return localStorage.getItem(name);
	} catch {
		return null;
	}
};

const setFlag = (name: string, value = "true"): void => {
	try {
		localStorage.setItem(name, value);
	} catch {
		// storage unavailable (private mode etc.) - flags just won't persist
	}
};

export const hasDoneGettingStarted = (): boolean =>
	getFlag(STORAGE_KEYS.gettingStartedDone) !== null;
export const markGettingStartedDone = (): void => setFlag(STORAGE_KEYS.gettingStartedDone);
export const setGettingStartedDone = (done: boolean): void => {
	// Monotonic: once the flag is set it is never cleared - deleting
	// your ideas must not resurrect onboarding.
	if (done) {
		setFlag(STORAGE_KEYS.gettingStartedDone);
	}
};
