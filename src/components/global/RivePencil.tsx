import { useRive, RuntimeLoader } from "@rive-app/react-canvas";

// Load the Rive WASM engine from the app bundle instead of the unpkg CDN
// (see scripts/vendor-assets.mjs) so animations work fully offline.
RuntimeLoader.setWasmUrl("/vendor/rive/rive.wasm");
RuntimeLoader.setWasmFallbackUrl("/vendor/rive/rive_fallback.wasm");

/**
 * The pencil mascot. Both animations mount ONCE and stay mounted; the
 * active one plays and the other pauses, so switching (every mic
 * click) never re-initializes the Rive state machines - it just
 * toggles play/pause on already-loaded instances.
 */
export default function RivePencil({
	type = "wave",
	small = false
}: {
	type?: "wave" | "jump";
	small?: boolean;
}) {
	return (
		<div className={small ? `w-24 h-28 relative` : `w-20 h-32 md:w-36 md:h-44 relative`}>
			<Wave active={type === "wave"} />
			<Jump active={type === "jump"} />
		</div>
	);
}

function Wave({ active }: { active: boolean }) {
	const { rive, RiveComponent } = useRive({
		src: "/rive/pencilwave_375_460.riv",
		autoplay: true
	});

	if (rive) {
		if (active) {
			rive.play();
		} else {
			rive.pause();
		}
	}

	return (
		<div className={active ? "" : "hidden"}>
			<RiveComponent
				onMouseEnter={() => rive && rive.play()}
				onMouseLeave={() => rive && rive.pause()}
			/>
		</div>
	);
}

function Jump({ active }: { active: boolean }) {
	const { rive, RiveComponent } = useRive({
		src: "/rive/penciljump.riv",
		autoplay: true
	});

	if (rive) {
		if (active) {
			rive.play();
		} else {
			rive.pause();
		}
	}

	return (
		<div className={active ? "" : "hidden"}>
			<RiveComponent />
		</div>
	);
}
