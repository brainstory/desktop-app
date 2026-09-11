import { map } from "nanostores";
import { getUserApi } from "@helpers/api/user";

/**
 * Initialize state as undefined.
 * If undefined consumers know api has not yet resolved to populate state
 */

/**
 * When populated, the userState will contain:
 * - name: Name of the current user
 * - createdAt: when the local account was created
 */
export const $userState = map({});
export const $userTrial = map({ isPaid: true });

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
						console.log("successful set timezone", userBrowserTimezone);
						$userState.setKey("timezone", userBrowserTimezone);
					})
					.catch((err) => console.log("unsuccessful at setting timezone", err));
			});
		}
	})
	.catch((err) => {
		console.log("error getting user data", err);
	});
