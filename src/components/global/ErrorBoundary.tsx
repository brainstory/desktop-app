import { Component, type ErrorInfo, type ReactNode } from "react";
import Button from "@ds/Button";
import ErrorSection from "@components/error/ErrorSection";

interface ErrorBoundaryProps {
	children: ReactNode;
	/** title of the fallback section */
	title?: string;
	/** the fallback's reload action; overridable so tests can observe the
	 * click without navigating (the default reloads the window) */
	onReload?: () => void;
}

const reloadWindow = () => window.location.reload();

interface ErrorBoundaryState {
	error: Error | null;
}

/**
 * Catches a render-time crash so the window shows a recoverable message
 * instead of going blank: every page is a `client:only` React island, so
 * a throw in one of its components otherwise leaves nothing behind but
 * an empty main section.
 */
export default class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
	override state: ErrorBoundaryState = { error: null };

	static getDerivedStateFromError(error: Error): ErrorBoundaryState {
		return { error };
	}

	override componentDidCatch(error: Error, info: ErrorInfo): void {
		console.error("render error caught by the boundary", error, info.componentStack);
	}

	override render(): ReactNode {
		const { error } = this.state;
		if (error !== null) {
			return (
				<ErrorSection
					title={this.props.title ?? "Something went wrong"}
					paragraphs={[
						"The page hit an unexpected error. Your ideas are safe on this device — reloading usually clears it.",
						`Details: ${error.message}`
					]}
					showRestartHint
					action={
						<Button variant="pink" onClick={this.props.onReload ?? reloadWindow}>
							Reload
						</Button>
					}
				/>
			);
		}
		return this.props.children;
	}
}
