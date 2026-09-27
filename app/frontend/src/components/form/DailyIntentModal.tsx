import { useState, useEffect, useRef } from "react";

import {
	getDailyLogQuestionsApi,
	submitDailyLogQuestionsApi
} from "@helpers/api/forms";
import type { LogFormAnswer } from "@helpers/api/forms";

import PinkButton from "@ds/PinkButton";
import ModalTitleBar from "@components/global/ModalTitleBar";
import LoadingAnimation from "@components/global/LoadingAnimation";
import StartLogSection from "./StartLogSection";

interface DailyIntentModalProps {
	logId?: string | null;
	setLogId: (id: string) => void;
	onClose: () => void;
	isAtStart?: boolean;
}

export default function DailyIntentModal({ setLogId, onClose }: DailyIntentModalProps) {
	const [isLogLoading, setIsLogLoading] = useState(true);
	const [logItems, setLogItems] = useState<LogFormAnswer[]>([]);
	const [isSaving, setIsSaving] = useState(false);
	const modalRef = useRef<HTMLDivElement | null>(null);

	useEffect(() => {
		getDailyLogQuestionsApi()
			.then((logQuestions) => {
				const updateLogItems = logQuestions.map((question: { id: number; text: string }) => ({
					id: question.id,
					text: question.text,
					value: false // default is always false
				}));
				setLogItems(updateLogItems);
				setIsLogLoading(false);
			})
			.catch((err) => {
				console.log("Error getting daily log questions", err);
				setIsLogLoading(false);
			});
	}, []);

	const onSubmit = () => {
		submitDailyLogQuestionsApi(logItems)
			.then((logId) => {
				onClose();
				setLogId(logId);
			})
			.catch((err) => {
				console.log("Error submitting daily log answers", err);
			})
			.finally(() => setIsSaving(false));
	};

	// Escape closes the modal; Tab is trapped inside it. Focus starts on
	// the first control and returns to the opener on close.
	useEffect(() => {
		const modal = modalRef.current;
		const previouslyFocused = document.activeElement as HTMLElement | null;

		const focusableSelector =
			'a[href], button:not([disabled]), textarea, input, select, [tabindex]:not([tabindex="-1"])';
		const focusables = (): HTMLElement[] =>
			Array.from(modal?.querySelectorAll<HTMLElement>(focusableSelector) ?? []);

		focusables()[0]?.focus();

		const onKeyDown = (event: globalThis.KeyboardEvent): void => {
			if (event.key === "Escape") {
				onClose();
				return;
			}
			if (event.key === "Tab") {
				const items = focusables();
				if (items.length === 0) return;
				const first = items[0];
				const last = items[items.length - 1];
				if (event.shiftKey && document.activeElement === first) {
					event.preventDefault();
					last.focus();
				} else if (!event.shiftKey && document.activeElement === last) {
					event.preventDefault();
					first.focus();
				}
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => {
			window.removeEventListener("keydown", onKeyDown);
			previouslyFocused?.focus();
		};
	}, [onClose]);

	return (
		<div
			role="dialog"
			aria-modal="true"
			aria-label="Daily Intent Log"
			className="fixed inset-0 z-50 flex items-center justify-center"
		>
			{/* backdrop: click to dismiss */}
			<div
				className="fixed inset-0 bg-black opacity-40"
				aria-hidden="true"
				onClick={onClose}
			></div>
			<div
				ref={modalRef}
				className="flex flex-col max-w-[600px] w-[90vw] max-h-[95vh] bg-white mx-auto rounded-lg relative shadow-md p-5 md:p-7"
				onClick={(e) => e.stopPropagation()}
			>
				<ModalTitleBar title="Daily Intent Log" onClose={onClose} classes="mb-2" />
				{isLogLoading ? (
					<div className="py-10">
						<LoadingAnimation text="Loading your daily questions..." />
					</div>
				) : (
					<>
						<StartLogSection
							logItems={logItems}
							setLogItems={setLogItems}
							disabled={isSaving}
						/>
						<PinkButton
							disabled={isSaving}
							onClick={() => {
								setIsSaving(true);
								onSubmit();
							}}
							classes="mx-auto mt-auto flex-end w-6em"
						>
							{isSaving && (
								<div className="animate-spin inline-block w-4 h-4 border-[2px] border-current border-t-transparent text-white rounded-full mr-2" />
							)}
							{isSaving ? "Saving" : "Submit"}
						</PinkButton>
					</>
				)}
			</div>
		</div>
	);
}
