import { map } from "nanostores";
import { getUserApi } from "@helpers/api/user";

/**
 * The userState store starts empty and is populated once getUserApi
 * resolves; empty means "not loaded yet". When populated it contains:
 * - userName: display name (may be undefined until the user sets one)
 * - createdAt: when the local profile was created
 * - timezone: the user's IANA timezone
 * (email/mailVerified were removed with the accounts they belonged to.)
 */
export const $userState = map({ loaded: false });

// Populate userState. Runs once at import; a failure must still mark the
// store loaded so consumers can render their empty states instead of
// waiting forever on a spinner.
getUserApi()
	.then((userRes) => {
		$userState.set({
			loaded: true,
			userName: userRes?.name,
			createdAt: userRes?.createdAt,
			timezone: userRes?.timezone
		});

		if (!userRes?.timezone) {
			const userBrowserTimezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
			import("@helpers/api/settings").then(({ saveUserSettingsApi }) => {
				saveUserSettingsApi(null, userBrowserTimezone)
					.then(() => {
						$userState.setKey("timezone", userBrowserTimezone);
					})
					.catch((err) => console.log("unsuccessful at setting timezone", err));
			});
		}
	})
	.catch((err) => {
		console.log("error getting user data", err);
		$userState.set({ loaded: true, userName: undefined, createdAt: undefined, timezone: undefined });
	});
