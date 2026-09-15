import { createContext, useContext, useState, type ReactNode } from "react";

export type SludgemanState = "idle" | "jump" | "wave";

interface AppContextValue {
	sludgeman: SludgemanState;
	setSludgeman: (state: SludgemanState) => void;
}

export const AppContext = createContext<AppContextValue>({ sludgeman: "idle", setSludgeman: () => {} });

export function AppWrapper({ children }: { children: ReactNode }) {
	const [sludgeman, setSludgeman] = useState<SludgemanState>("idle");
	const sharedState: AppContextValue = {
		sludgeman,
		setSludgeman
	};

	return <AppContext.Provider value={sharedState}>{children}</AppContext.Provider>;
}

export function useAppContext(): AppContextValue {
	return useContext(AppContext);
}
