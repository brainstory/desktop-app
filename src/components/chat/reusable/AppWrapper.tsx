import { createContext, useContext, useState, type ReactNode } from "react";

export type SludgemanState = "idle" | "jump" | "wave";

interface AppContextValue {
	sludgeman: SludgemanState;
	setSludgeman: (state: SludgemanState) => void;
}

export const AppContext = createContext<AppContextValue>({
	sludgeman: "idle",
	setSludgeman: () => {}
});

export function AppWrapper({ children }: { children: ReactNode }) {
	const [sludgeman, setSludgeman] = useState<SludgemanState>("idle");
	// the AI status feed ($aiStatus) starts itself when a consumer
	// subscribes; nothing to initialise here
	const sharedState: AppContextValue = {
		sludgeman,
		setSludgeman
	};

	return <AppContext.Provider value={sharedState}>{children}</AppContext.Provider>;
}

export function useAppContext(): AppContextValue {
	return useContext(AppContext);
}
