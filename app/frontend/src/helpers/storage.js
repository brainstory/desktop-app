// Simple persisted flags for onboarding state. Cookies turned out to be
// unreliable in the Tauri webview (custom scheme + Secure cookies), so
// local storage is the durable mechanism in the desktop app.

const getFlag = (name) => {
	try {
		return localStorage.getItem(name);
	} catch {
		return null;
	}
};

const setFlag = (name, value = "true") => {
	try {
		localStorage.setItem(name, value);
	} catch {
		// storage unavailable (private mode etc.) - flags just won't persist
	}
};

export const hasDoneGettingStarted = () => getFlag("has_done_getting_started") !== null;
export const markGettingStartedDone = () => setFlag("has_done_getting_started");
export const setGettingStartedDone = (done) => {
	if (done) {
		setFlag("has_done_getting_started");
	} else {
		try {
			localStorage.removeItem("has_done_getting_started");
		} catch {
			// ignore
		}
	}
};
export const hasSeenIndex = () => getFlag("has_seen_index") !== null;
export const markIndexSeen = () => setFlag("has_seen_index");
