// Simple persisted flags for onboarding state. Cookies turned out to be
// unreliable in the Tauri webview (custom scheme + Secure cookies), so
// local storage is the durable mechanism in the desktop app.

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

export const hasDoneGettingStarted = (): boolean => getFlag("has_done_getting_started") !== null;
export const markGettingStartedDone = (): void => setFlag("has_done_getting_started");
export const setGettingStartedDone = (done: boolean): void => {
	// Monotonic: once the flag is set it is never cleared - deleting
	// your ideas must not resurrect onboarding.
	if (done) {
		setFlag("has_done_getting_started");
	}
};
