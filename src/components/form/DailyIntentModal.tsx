import { useState, useEffect, useRef } from "react";

import { getDailyLogQuestionsApi, submitDailyLogQuestionsApi } from "@helpers/api/forms";
import type { LogFormAnswer } from "@helpers/api/forms";
import { normalizeApiError } from "@helpers/helpers";

import Button from "@ds/Button";
import ModalTitleBar from "@components/global/ModalTitleBar";
import LoadingAnimation from "@components/global/LoadingAnimation";
import StartLogSection from "./StartLogSection";

interface DailyIntentModalProps {
	setLogId: (id: string) => void;
	onClose: () => void;
}

export default function DailyIntentModal({ setLogId, onClose }: DailyIntentModalProps) {
	const [isLogLoading, setIsLogLoading] = useState(true);
	const [loadError, setLoadError] = useState<string | null>(null);
	const [loadAttempt, setLoadAttempt] = useState(0);
	const [logItems, setLogItems] = useState<LogFormAnswer[]>([]);
	const [isSaving, setIsSaving] = useState(false);
	const [saveError, setSaveError] = useState<string | null>(null);
	const modalRef = useRef<HTMLDivElement | null>(null);
	const retryButtonRef = useRef<HTMLButtonElement | null>(null);

	// A load or save that settles after the modal closed (or after a newer
	// retry started) must not touch state: isMountedRef gates saves, and
	// each load bumping loadSeqRef invalidates every earlier in-flight
	// load on close/retry.
	const isMountedRef = useRef(true);
	const loadSeqRef = useRef(0);
	useEffect(() => {
		isMountedRef.current = true;
		return () => {
			isMountedRef.current = false;
		};
	}, []);

	useEffect(() => {
		// the bump invalidates any earlier in-flight load (a retry or a
		// remount runs this effect again); the mount guard covers close
		const seq = ++loadSeqRef.current;
		getDailyLogQuestionsApi()
			.then((logQuestions) => {
				if (!isMountedRef.current || seq !== loadSeqRef.current) return;
				setLogItems(
					logQuestions.map((question) => ({
						id: question.id,
						text: question.text,
						value: question.value ?? false // false only when absent
					}))
				);
				setLoadError(null);
				setIsLogLoading(false);
			})
			.catch((err) => {
				if (!isMountedRef.current || seq !== loadSeqRef.current) return;
				console.error("Error getting daily log questions", err);
				setLoadError(normalizeApiError(err));
				setIsLogLoading(false);
			});
	}, [loadAttempt]);

	// A failed load is retried by remounting the effect with a new attempt.
	const onRetryLoad = () => {
		setLoadError(null);
		setIsLogLoading(true);
		setLoadAttempt((attempt) => attempt + 1);
	};

	useEffect(() => {
		// land keyboard users on the recovery action; the role="alert"
		// banner announces the failure itself
		if (loadError !== null) retryButtonRef.current?.focus();
	}, [loadError]);

	// belt and braces beyond the disabled button: a second activation
	// while a save is pending must not submit again
	const isSavingRef = useRef(false);
	const onSubmit = () => {
		if (isSavingRef.current) return;
		isSavingRef.current = true;
		setSaveError(null);
		setIsSaving(true);
		submitDailyLogQuestionsApi(logItems)
			.then((logId) => {
				if (!isMountedRef.current) return; // closed while saving
				onClose();
				setLogId(logId);
			})
			.catch((err) => {
				console.error("Error submitting daily log answers", err);
				if (!isMountedRef.current) return;
				// keep the entered answers so the user can simply retry
				setSaveError(normalizeApiError(err));
			})
			.finally(() => {
				isSavingRef.current = false;
				if (isMountedRef.current) setIsSaving(false);
			});
	};

	// Escape closes the modal; Tab is trapped inside it. Focus starts on
	// the first control and returns to the opener on close.
	// the close handler is read through a ref so the trap effect runs
	// exactly once: re-running it on every parent render re-captured
	// "previously focused" from inside the modal
	const onCloseRef = useRef(onClose);
	useEffect(() => {
		onCloseRef.current = onClose;
	}, [onClose]);

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
				onCloseRef.current();
				return;
			}
			if (event.key === "Tab") {
				const items = focusables();
				if (items.length === 0) return;
				const first = items[0]!;
				const last = items[items.length - 1]!;
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
	}, []);

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
			>
				<ModalTitleBar title="Daily Intent Log" onClose={onClose} classes="mb-2" />
				{isLogLoading ? (
					<div className="py-10">
						<LoadingAnimation text="Loading your daily questions..." />
					</div>
				) : loadError !== null ? (
					// a failed load shows the error and a retry instead of the
					// form: an empty answer list must never be submittable as
					// if the load had succeeded
					<div
						role="alert"
						className="flex flex-col items-start gap-4 border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 text-sm my-5"
					>
						<p>
							<b>Couldn’t load your daily questions.</b> {loadError} Nothing was saved
							— try again.
						</p>
						<Button variant="pink" ref={retryButtonRef} onClick={onRetryLoad}>
							Try again
						</Button>
					</div>
				) : (
					<>
						{saveError !== null && (
							<div
								role="alert"
								className="border border-amber-300 bg-amber-50 text-amber-900 rounded-lg p-4 text-sm mb-2"
							>
								<b>Couldn’t save your daily log.</b> {saveError} Your answers are
								still here — press Submit to try again.
							</div>
						)}
						<StartLogSection
							logItems={logItems}
							setLogItems={setLogItems}
							disabled={isSaving}
						/>
						<Button
							variant="pink"
							disabled={isSaving}
							onClick={onSubmit}
							classes="mx-auto mt-auto justify-end w-[12rem]"
						>
							{isSaving && (
								<div className="animate-spin inline-block w-4 h-4 border-[2px] border-current border-t-transparent text-white rounded-full mr-2" />
							)}
							{isSaving ? "Saving" : "Submit"}
						</Button>
					</>
				)}
			</div>
		</div>
	);
}
