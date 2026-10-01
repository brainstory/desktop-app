import { useState, useEffect } from "react";
import { useStore } from "@nanostores/react";
import { $userState } from "@components/global/userStore";

import { getUserSettingsApi, saveUserSettingsApi } from "@helpers/api/settings";
import type { LogSettingsQuestion, NotificationSetting } from "@helpers/api/settings";
import { getQueryParam, normalizeApiError } from "@helpers/helpers";

import { PhotoNameCard, GeneralCard } from "./ProfileCards";
import { DailyLogSettingsCard } from "./DailyLogSettingsCard";
import { NotificationsCard } from "./NotificationsCard";
import AiModelsCard from "./AiModelsCard";
import AppPresenceCard from "./AppPresenceCard";
import UpdatesCard from "./UpdatesCard";

import LoadingAnimation from "@components/global/LoadingAnimation";
import ErrorSection from "@components/error/ErrorSection";
import PinkButton from "@ds/PinkButton";
import { Snackbar, ERROR_COPY, SUCCESS_COPY } from "@ds/Snackbar";
import { TailwindComposedTabs } from "@ds/TailwindTabs";

const TAB_MAP: Record<string, number> = {
	general: 0,
	dailyLog: 1,
	aiModels: 2
};

export default function Profile() {
	const userState = useStore($userState);
	const { createdAt } = userState;

	const [activeTab] = useState<number>(() => {
		const tab = getQueryParam("tab");
		return tab ? (TAB_MAP[tab] ?? 0) : 0;
	});
	const [userName, setUserName] = useState<string>("");
	const [userTimezone, setUserTimezone] = useState("");
	const [notifications, setNotifications] = useState<NotificationSetting[]>([]);
	const [dailyLogSettings, setDailyLogSettings] = useState<LogSettingsQuestion[]>([]);
	const [presence, setPresence] = useState({ dock: true, tray: true });
	const [updatesEnabled, setUpdatesEnabled] = useState(true);
	const [snackbarSuccessOpen, setSnackbarSuccessOpen] = useState(false);
	const [snackbarSuccessMessage, setSnackbarSuccessMessage] = useState(SUCCESS_COPY.DEFAULT);
	const [snackbarErrorOpen, setSnackbarErrorOpen] = useState(false);
	const [snackbarErrorMessage, setSnackbarErrorMessage] = useState(ERROR_COPY.DEFAULT);
	const [isLoading, setIsLoading] = useState(true);
	const [errorFound, setErrorFound] = useState(false);

	const loadSettings = () => {
		getUserSettingsApi()
			.then((res) => {
				setUserName(res.user.name ?? "");
				setUserTimezone(res.user.timezone ? res.user.timezone : "Etc/GMT");
				setDailyLogSettings(res.dailyLog);
				setNotifications(res.notifications);
				setPresence(res.presence);
				setUpdatesEnabled(res.updates.enabled);
			})
			.catch((e) => {
				// without this the spinner never ends
				console.error("error getting user settings", e);
				setErrorFound(true);
			})
			.finally(() => setIsLoading(false));
	};

	/** retry path: re-arm the loading state (a click handler may set state
	 * synchronously; the mount effect may not) */
	const retryLoadSettings = () => {
		setIsLoading(true);
		setErrorFound(false);
		loadSettings();
	};

	useEffect(() => {
		loadSettings();
	}, []);

	const openSnackbar = (isSuccess: boolean, message: string): void => {
		if (isSuccess) {
			setSnackbarSuccessOpen(true);
			setSnackbarSuccessMessage(message);
		} else {
			setSnackbarErrorOpen(true);
			setSnackbarErrorMessage(message);
		}
	};

	const handleUserSettingsSave = (newName: string, newTimezone: string): void => {
		saveUserSettingsApi({ name: newName, timezone: newTimezone })
			.then(() => {
				loadSettings();
				openSnackbar(true, SUCCESS_COPY.SAVE);
			})
			.catch((e) => {
				openSnackbar(false, normalizeApiError(e));
			});
	};

	const handleNotificationsSave = (notificationFields: NotificationSetting[]): Promise<boolean> =>
		saveUserSettingsApi({
			notifications: notificationFields as unknown as Record<string, unknown>[]
		})
			.then(() => {
				openSnackbar(true, SUCCESS_COPY.SAVE);
				return true;
			})
			.catch((e) => {
				openSnackbar(false, normalizeApiError(e));
				return false;
			});

	const handleLogSettingsSave = (enabledLogQids: number[]): void => {
		// copy before sorting (the prop is the child's state) and sort
		// numerically - lexicographic sort puts 10 before 2
		const sortedEnabledLogQids = [...enabledLogQids].sort((a, b) => a - b);
		saveUserSettingsApi({ enabledLogQids: sortedEnabledLogQids })
			.then(() => {
				openSnackbar(true, SUCCESS_COPY.SAVE);
			})
			.catch((e) => {
				openSnackbar(false, normalizeApiError(e));
			});
	};

	const tabData = [
		{
			label: "General",
			content: (
				<div className="grid grid-cols-3 gap-4 divide-y divide-stone-200">
					<PhotoNameCard
						userName={userName}
						createdAt={createdAt}
						openSnackbar={openSnackbar}
					/>
					<GeneralCard
						userName={userName}
						timezone={userTimezone}
						saveSettings={handleUserSettingsSave}
					/>
					<NotificationsCard
						key={userTimezone} // rerender when timezone changes
						notificationsData={notifications}
						saveSettings={handleNotificationsSave}
					/>
					<AppPresenceCard presence={presence} openSnackbar={openSnackbar} />
					<UpdatesCard enabled={updatesEnabled} openSnackbar={openSnackbar} />
				</div>
			)
		},
		{
			label: "Daily Log Settings",
			content: (
				<DailyLogSettingsCard
					logFieldsData={dailyLogSettings}
					saveSettings={handleLogSettingsSave}
				/>
			)
		},
		{
			label: "AI Models",
			content: <AiModelsCard openSnackbar={openSnackbar} />
		}
	];

	return (
		<div className="flex flex-col items-center">
			{snackbarSuccessOpen && (
				<Snackbar
					isSuccess={true}
					message={snackbarSuccessMessage}
					onClose={() => setSnackbarSuccessOpen(false)}
				/>
			)}
			{snackbarErrorOpen && (
				<Snackbar
					isSuccess={false}
					message={snackbarErrorMessage}
					onClose={() => setSnackbarErrorOpen(false)}
				/>
			)}

			{isLoading ? (
				<LoadingAnimation text="Loading your settings..." />
			) : errorFound ? (
				<ErrorSection
					title="Couldn't load your settings"
					paragraphs={["Something went wrong while loading your settings."]}
					action={<PinkButton onClick={retryLoadSettings}>Try again</PinkButton>}
					hideDashboardLink
				/>
			) : (
				<TailwindComposedTabs
					data={tabData}
					activeTab={activeTab}
					accentColor="pink"
					tabParams={["general", "dailyLog", "aiModels"]}
				/>
			)}
		</div>
	);
}
