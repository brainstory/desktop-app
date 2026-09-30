// Literal class strings: Tailwind v4 only emits the utilities it can
// see at build time, so h-${size} templates never resolved to anything.
const SIZE_CLASSES: Record<string, string> = {
	"4": "h-4 w-4",
	"6": "h-6 w-6",
	"8": "h-8 w-8",
	"10": "h-10 w-10",
	"12": "h-12 w-12",
	"14": "h-14 w-14"
};

export default function Avatar({
	id,
	charToShow,
	size = "10",
	style = ""
}: {
	id?: string | null;
	charToShow?: string;
	size?: number | string;
	style?: string;
}) {
	const sizeClasses = SIZE_CLASSES[String(size)] ?? SIZE_CLASSES["10"];
	const containerClasses = `${sizeClasses} relative inline-flex items-center justify-center overflow-hidden rounded-full ${style} `;

	return (
		<div className={containerClasses + getColorFromId(id ?? "?")}>
			<span className="capitalize font-medium text-white text-sm">{charToShow}</span>
		</div>
	);
}

const bgColors = [
	"bg-slate-600",
	"bg-red-700",
	"bg-pink-600",
	"bg-amber-600",
	"bg-emerald-600",
	"bg-green-800",
	"bg-lime-700",
	"bg-cyan-600",
	"bg-blue-800",
	"bg-indigo-700",
	"bg-violet-800",
	"bg-fuchsia-700",
	"bg-rose-700"
];

function getColorFromId(id: string): string {
	let hash = 0,
		i,
		chr;
	for (i = 0; i < id.length; i++) {
		chr = id.charCodeAt(i);
		hash = (hash << 5) - hash + chr;
		hash |= 0; // Convert to 32bit integer
	}
	const index = Math.abs(hash) % bgColors.length;
	return bgColors[index];
}
