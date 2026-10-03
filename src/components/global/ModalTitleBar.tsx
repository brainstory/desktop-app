import Button from "@ds/Button";

export default function ModalTitleBar({
	title,
	onClose,
	onBack = null,
	classes = ""
}: {
	title: string;
	onClose: () => void;
	onBack?: (() => void) | null;
	classes?: string;
}) {
	return (
		<div className={`flex flex-row items-start justify-between ${classes}`}>
			<span className="flex flex-row items-start">
				{onBack && (
					<Button
						variant="transparent"
						onClick={onBack}
						classes="w-8	h-8 mr-3"
						icon="arrow-back"
						sr="Go back to more share options"
					/>
				)}
				<h2 className="mr-3 text-xl font-semibold text-gray-900">{title}</h2>
			</span>
			<Button
				variant="transparent"
				onClick={onClose}
				classes="w-8 h-8"
				icon="close"
				sr="Close modal"
			/>
		</div>
	);
}
