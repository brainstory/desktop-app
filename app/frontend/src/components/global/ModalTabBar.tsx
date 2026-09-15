import Tooltip from "@ds/Tooltip";

const unSelectedTab = "flex-1";
const selectedTab = "flex-1 bg-white rounded-lg font-semibold";

interface ModalTabBarProps {
	allTabs?: string[];
	currentTab?: string;
	disabledTabs?: string[];
	setTab: (tab: string) => void;
	classes?: string;
}

export default function ModalTabBar({
	allTabs = [],
	currentTab,
	disabledTabs = [],
	setTab,
	classes = ""
}: ModalTabBarProps) {
	return (
		<div
			role="tablist"
			className={`flex text-sm sm:text-base rounded-lg border border-stone-300 p-0.5 bg-stone-200 ${classes}`}
		>
			{allTabs.map((tabItem) => {
				const isDisabled = disabledTabs.includes(tabItem);
				let tabStyle = unSelectedTab;
				if (currentTab === tabItem) {
					tabStyle = selectedTab;
				} else if (isDisabled) {
					tabStyle = unSelectedTab + " text-stone-500";
				}
				return (
					<button
						key={tabItem}
						role="tab"
						aria-selected={currentTab === tabItem}
						disabled={isDisabled}
						className={tabStyle}
						onClick={() => {
							setTab(tabItem);
						}}
					>
						{isDisabled ? (
							<Tooltip
								text="Finish setting daily log and intent"
								classes="justify-center items-center gap-2"
							>
								<ion-icon aria-hidden="true" name="lock-closed-outline"></ion-icon>
								<p>{tabItem}</p>
							</Tooltip>
						) : (
							<p>{tabItem}</p>
						)}
					</button>
				);
			})}
		</div>
	);
}
