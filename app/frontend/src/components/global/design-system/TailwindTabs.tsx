import {
	useState,
	createContext,
	useContext,
	Children,
	type ReactNode,
	type ComponentProps
} from "react";

/*

Usage (or look at TailwindComposedTabs):

<TailwindTabs>
	<TailwindTabList>
		{data.map(tab => (
			<TailwindTab>{tab.label}</TailwindTab>
		))}
	</TailwindTabList>
	<TailwindTabPanels>
		{data.map(tab => (
			<TailwindTabPanel>{tab.content}</TailwindTabPanel>
		))}
	</TailwindTabPanels>
</TailwindTabs>

*/

interface TabsContextValue {
	activeIndex: number;
	setActiveIndex: (index: number) => void;
	tabCount: number;
}

const TabsContext = createContext<TabsContextValue | undefined>(undefined);

interface TailwindTabsProps {
	children: ReactNode;
	activeTab?: number;
	/** Values for the ?tab= URL param, one per tab; keeps deep links alive on click/back. */
	tabParams?: string[];
}

function TailwindTabs({ children, activeTab = 0, tabParams }: TailwindTabsProps) {
	// Clamp to the real tab count: an activeTab from a ?tab= deep link
	// that this tab set doesn't have (e.g. "feedback" on a feedback idea,
	// which has no feedback tab) must land on a real panel, not render
	// nothing.
	const tabCount = Children.count(children);
	const clamp = (index: number) => Math.max(0, Math.min(index, tabCount - 1));
	const [activeIndex, setActiveIndexRaw] = useState(() => clamp(activeTab));

	const setActiveIndex = (index: number) => {
		const next = clamp(index);
		setActiveIndexRaw(next);
		const param = tabParams?.[next];
		if (param) {
			const url = new URL(window.location.href);
			url.searchParams.set("tab", param);
			history.replaceState(null, "", url);
		}
	};

	return (
		<TabsContext.Provider
			value={{ activeIndex, setActiveIndex, tabCount: Children.count(children) }}
		>
			<div className="h-full">{children}</div>
		</TabsContext.Provider>
	);
}

const TabContext = createContext<number>(0);

function TailwindTabList({ children }: { children: ReactNode }) {
	const wrappedChildren = Children.map(children, (child, index) => (
		<TabContext.Provider value={index}>{child}</TabContext.Provider>
	));
	return (
		<div role="tablist" className="flex flex-wrap justify-center mb-6 gap-1">
			{wrappedChildren}
		</div>
	);
}

function TailwindTab({
	children,
	isDisabled,
	tooltipText = "",
	...rest
}: {
	children?: ReactNode;
	isDisabled?: boolean;
	tooltipText?: string;
} & Omit<ComponentProps<"button">, "children">) {
	const index = useContext(TabContext);
	const ctx = useContext(TabsContext);
	const activeIndex = ctx?.activeIndex ?? 0;
	const setActiveIndex = ctx?.setActiveIndex ?? (() => {});
	const tabCount = ctx?.tabCount ?? 1;
	const isActive = index === activeIndex;

	const moveFocus = (nextIndex: number) => {
		// roving tabindex: focus (and select) the sibling tab button by id
		const sibling = document.getElementById(`tw-tab-${nextIndex}`);
		sibling?.focus();
		sibling?.click();
	};

	const handleKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>): void => {
		switch (event.key) {
			case "ArrowRight":
				event.preventDefault();
				moveFocus((index + 1) % tabCount);
				break;
			case "ArrowLeft":
				event.preventDefault();
				moveFocus((index - 1 + tabCount) % tabCount);
				break;
			case "Home":
				event.preventDefault();
				moveFocus(0);
				break;
			case "End":
				event.preventDefault();
				moveFocus(tabCount - 1);
				break;
		}
	};

	return (
		<div className="group inline-block relative">
			<button
				id={`tw-tab-${index}`}
				type="button"
				role="tab"
				aria-selected={isActive}
				aria-controls={`tw-tabpanel-${index}`}
				tabIndex={isActive ? 0 : -1}
				disabled={isDisabled}
				className={`cursor-pointer text-sm font-medium bg-white p-3 border-b-4 focus-visible:ring-4 focus-visible:outline-none focus-visible:ring-pink-300 ${
					isDisabled
						? "border-stone-200 opacity-50 cursor-not-allowed"
						: isActive
							? `active border-pink-300`
							: "border-stone-200 hover:text-pink-600 hover:border-accent-600"
				}`}
				onClick={isDisabled ? undefined : () => setActiveIndex(index)}
				onKeyDown={handleKeyDown}
				{...rest}
			>
				{children}
			</button>
			{tooltipText !== "" && (
				<div
					role="tooltip"
					className="w-full text-center pointer-events-none absolute top-full left-1/2 transform -translate-x-1/2 p-2 bg-stone-800 text-white text-sm rounded opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-opacity duration-300"
				>
					{tooltipText}
				</div>
			)}
		</div>
	);
}

function TailwindTabPanels({ children }: { children: ReactNode }) {
	const activeIndex = useContext(TabsContext)?.activeIndex ?? 0;
	return Children.toArray(children)[activeIndex];
}

function TailwindTabPanel({ children }: { children: ReactNode }) {
	return children;
}

interface ComposedTab {
	label: string;
	content: ReactNode;
	tooltipText?: string;
	disabled?: boolean;
}

function TailwindComposedTabs({
	data,
	activeTab = 0,
	tabParams
}: {
	data: ComposedTab[];
	activeTab?: number;
	accentColor?: string;
	tabParams?: string[];
}) {
	return (
		<TailwindTabs activeTab={activeTab} tabParams={tabParams}>
			<TailwindTabList>
				{data.map((tab, i) => (
					<TailwindTab
						isDisabled={tab.disabled}
						tooltipText={tab.tooltipText ? tab.tooltipText : ""}
						key={`tw-tab-${i}`}
					>
						{tab.label}
					</TailwindTab>
				))}
			</TailwindTabList>
			<TailwindTabPanels>
				{data.map((tab, i) => (
					<TailwindTabPanel key={`tw-tabp-${i}`}>
						{/* only the active panel is in the DOM, so ids stay unique */}
						<div
							id={`tw-tabpanel-${i}`}
							role="tabpanel"
							aria-labelledby={`tw-tab-${i}`}
							className="outline-none"
						>
							{tab.content}
						</div>
					</TailwindTabPanel>
				))}
			</TailwindTabPanels>
		</TailwindTabs>
	);
}

export {
	TailwindTabs,
	TailwindTabList,
	TailwindTab,
	TailwindTabPanels,
	TailwindTabPanel,
	TailwindComposedTabs
};
