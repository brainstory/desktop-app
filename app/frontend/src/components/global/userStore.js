import { map } from "nanostores";
import { getUserApi } from "@helpers/api/user";

/**
 * The userState store starts empty and is populated once getUserApi
 * resolves; empty means "not loaded yet".
 *
 * When populated, it contains:
 * - name: Name of the current user
 * - createdAt: when the local account was created
 */
export const $userState = map({});

// Populate userState
getUserApi()
	.then((userRes) => {
		$userState.set({
			userName: userRes?.name,
			userEmail: userRes?.email,
			userMailVerified: userRes?.mailVerified,
			createdAt: userRes?.createdAt,
			timezone: userRes?.timezone
		});

		if (!userRes.timezone) {
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
	});
