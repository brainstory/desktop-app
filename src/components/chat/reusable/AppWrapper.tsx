import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import ErrorBoundary from "@components/global/ErrorBoundary";
import { useSnackbar } from "@ds/Snackbar";

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
	const { openSnackbar, snackbars } = useSnackbar();
	// the AI status feed ($aiStatus) starts itself when a consumer
	// subscribes; nothing to initialise here

	// The quick-capture shortcut refused to navigate because a recording
	// or generation is in progress: the window came up, so explain why
	// no new chat opened.
	useEffect(() => {
		let unlisten: (() => void) | undefined;
		void listen("quick-capture-blocked", () => {
			openSnackbar(false, "Finish or stop the current recording first.");
		}).then((fn) => {
			unlisten = fn;
		});
		return () => unlisten?.();
	}, [openSnackbar]);

	const sharedState: AppContextValue = {
		sludgeman,
		setSludgeman
	};

	return (
		<AppContext.Provider value={sharedState}>
			<ErrorBoundary title="This session hit a snag">{children}</ErrorBoundary>
			{snackbars}
		</AppContext.Provider>
	);
}

export function useAppContext(): AppContextValue {
	return useContext(AppContext);
}
