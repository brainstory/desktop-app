import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { initAiStatus } from "@components/global/aiStatusStore";

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
	// one runtime-status read + event subscription per app, shared by
	// every consumer of $aiStatus
	useEffect(() => {
		initAiStatus();
	}, []);
	const sharedState: AppContextValue = {
		sludgeman,
		setSludgeman
	};

	return <AppContext.Provider value={sharedState}>{children}</AppContext.Provider>;
}

export function useAppContext(): AppContextValue {
	return useContext(AppContext);
}
