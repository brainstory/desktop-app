export default function ChangeInputTypeButton({
	isTextInput,
	onToggle
}: {
	isTextInput: boolean;
	onToggle: (e?: unknown) => void;
}) {
	const buttonStyle = "text-stone-500 text-sm underline underline-offset-2 hover:no-underline";

	// data-testid: the e2e smoke test switches the recorder to typing
	if (isTextInput) {
		return (
			<button
				className={buttonStyle}
				onClick={() => onToggle()}
				data-testid="text-mode-toggle"
			>
				Use the microphone
			</button>
		);
	} else {
		return (
			<button
				className={buttonStyle}
				onClick={() => onToggle()}
				data-testid="text-mode-toggle"
			>
				Type instead
			</button>
		);
	}
}
