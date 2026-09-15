import { useState, createContext, useContext, Children, type ReactNode, type ComponentProps } from "react";

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
}

const TabsContext = createContext<TabsContextValue | undefined>(undefined);

interface TailwindTabsProps {
	children: ReactNode;
	activeTab?: number;
}

function TailwindTabs({ children, activeTab = 0 }: TailwindTabsProps) {
	const [activeIndex, setActiveIndex] = useState(activeTab);
	return (
		<TabsContext.Provider value={{ activeIndex, setActiveIndex }}>
			<div className="h-full">{children}</div>
		</TabsContext.Provider>
	);
}

const TabContext = createContext<number>(0);

function TailwindTabList({ children }: { children: ReactNode }) {
	const wrappedChildren = Children.map(children, (child, index) => (
		<TabContext.Provider value={index}>{child}</TabContext.Provider>
	));
	return <ul className="flex flex-wrap justify-center mb-6 gap-1">{wrappedChildren}</ul>;
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
} & Omit<ComponentProps<"li">, "children">) {
	const index = useContext(TabContext);
	const ctx = useContext(TabsContext);
	const activeIndex = ctx?.activeIndex ?? 0;
	const setActiveIndex = ctx?.setActiveIndex ?? (() => {});
	const isActive = index === activeIndex;

	return (
		<div className="group inline-block relative">
			<li
				className={`cursor-pointer text-sm font-medium bg-white p-3 border-b-4 ${
					isDisabled
						? "disabled border-stone-200 opacity-50 cursor-not-allowed hover:unset"
						: isActive
						? `active border-pink-300`
						: "border-stone-200 hover:text-pink-600 hover:border-accent-900"
				}`}
				onClick={isDisabled ? undefined : () => setActiveIndex(index)}
				key={index + "tab"}
				{...rest}
			>
				{children}
			</li>
			{tooltipText !== "" && (
				<div className="w-full text-center pointer-events-none absolute top-full left-1/2 transform -translate-x-1/2 p-2 bg-stone-800 text-white text-sm rounded opacity-0 group-hover:opacity-100 transition-opacity duration-300">
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
	activeTab = 0
}: {
	data: ComposedTab[];
	activeTab?: number;
	accentColor?: string;
}) {
	return (
		<TailwindTabs activeTab={activeTab}>
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
					<TailwindTabPanel key={`tw-tabp-${i}`}>{tab.content}</TailwindTabPanel>
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
