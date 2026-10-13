import { map, onMount } from "nanostores";
import { getUserApi } from "@helpers/api/user";

export interface UserState {
	loaded: boolean;
	userName?: string | null;
	createdAt?: string | null;
	timezone?: string | null;
}

/**
 * The userState store starts as { loaded: false } and is filled once
 * getUserApi settles (loaded: true, even when the call failed). Then it
 * holds:
 * - userName: display name (may be undefined until the user sets one)
 * - createdAt: when the local profile was created
 * - timezone: the user's IANA timezone
 *
 * The read starts lazily on the first subscriber (and restarts after the
 * last one leaves), instead of firing at module import: importing a
 * component in a test (or any future non-UI context) no longer performs
 * IPC as a side effect, and the data is fetched fresh per page.
 */
export const $userState = map<UserState>({ loaded: false });

/** bumped on teardown so an in-flight read cannot write into a new lifecycle */
let generation = 0;

function loadUser(): void {
	const gen = generation;
	getUserApi()
		.then((userRes) => {
			if (gen !== generation) return;
			$userState.set({
				loaded: true,
				userName: userRes?.name,
				createdAt: userRes?.createdAt,
				timezone: userRes?.timezone
			});

			if (!userRes?.timezone) {
				const userBrowserTimezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
				import("@helpers/api/settings")
					.then(({ saveUserSettingsApi }) =>
						saveUserSettingsApi({ timezone: userBrowserTimezone })
					)
					.then(() => {
						if (gen !== generation) return;
						$userState.setKey("timezone", userBrowserTimezone);
					})
					.catch((err) => console.error("unsuccessful at setting timezone", err));
			}
		})
		.catch((err) => {
			if (gen !== generation) return;
			// a failure must still mark the store loaded so consumers can
			// render their empty states instead of waiting forever
			console.error("error getting user data", err);
			$userState.set({
				loaded: true,
				userName: undefined,
				createdAt: undefined,
				timezone: undefined
			});
		});
}

onMount($userState, () => {
	loadUser();
	return () => {
		generation += 1;
		// nobody is listening: the next subscriber starts a fresh read
		$userState.set({ loaded: false });
	};
});
