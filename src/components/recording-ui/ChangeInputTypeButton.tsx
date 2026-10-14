export default function ChangeInputTypeButton({
	isTextInput,
	onToggle
}: {
	isTextInput: boolean;
	onToggle: (e?: unknown) => void;
}) {
	const buttonStyle = "text-stone-500 text-sm underline underline-offset-2 hover:no-underline";

	if (isTextInput) {
		return (
			<button className={buttonStyle} onClick={() => onToggle()}>
				Use the microphone
			</button>
		);
	} else {
		return (
			<button className={buttonStyle} onClick={() => onToggle()}>
				Type instead
			</button>
		);
	}
}
