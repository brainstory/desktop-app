import { useEffect } from "react";
import { useRive, RuntimeLoader } from "@rive-app/react-canvas";
import { useMediaQuery } from "@src/hooks/useMediaQuery";

// Load the Rive WASM engine from the app bundle instead of the unpkg CDN
// (see scripts/vendor-assets.mjs) so animations work fully offline. The
// fallback slot points at the same engine: the webview is always a known
// modern WebAssembly host, so shipping the separate 2.1 MB
// non-SIMD/threads fallback build bought nothing.
RuntimeLoader.setWasmUrl("/vendor/rive/rive.wasm");
RuntimeLoader.setWasmFallbackUrl("/vendor/rive/rive.wasm");

/**
 * The pencil mascot. Both animations mount ONCE and stay mounted; the
 * active one plays and the other pauses, so switching (every mic
 * click) never re-initializes the Rive state machines - it just
 * toggles play/pause on already-loaded instances. Both canvases fill
 * the (sized) parent absolutely; the inactive one is faded out rather
 * than display:none, because a hidden canvas reports zero size and
 * stops rendering when it comes back.
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

/** Shared layer wrapper: fills the parent; faded out + click-through
 * when inactive (kept laid out so the canvas keeps its size). */
function Layer({ active, children }: { active: boolean; children: React.ReactNode }) {
	return (
		<div
			className={`absolute inset-0 transition-opacity duration-150 ${
				active ? "opacity-100" : "opacity-0 pointer-events-none"
			}`}
			// the whole mascot is decoration: hidden from the a11y tree
			// whether playing or not
			aria-hidden="true"
		>
			{children}
		</div>
	);
}

// the mascot is decorative: reduced-motion users get the static first
// frame instead of autoplaying animations
function useReducedMotion(): boolean {
	return useMediaQuery("(prefers-reduced-motion: reduce)");
}

function Wave({ active }: { active: boolean }) {
	const reduceMotion = useReducedMotion();
	const { rive, RiveComponent } = useRive({
		src: "/rive/pencilwave_375_460.riv",
		autoplay: !reduceMotion
	});

	useEffect(() => {
		if (!rive) return;
		if (active && !reduceMotion) {
			rive.play();
		} else {
			rive.pause();
		}
	}, [rive, active, reduceMotion]);

	return (
		<Layer active={active}>
			<RiveComponent
				onMouseEnter={() => {
					if (rive && !reduceMotion) rive.play();
				}}
				onMouseLeave={() => rive && rive.pause()}
			/>
		</Layer>
	);
}

function Jump({ active }: { active: boolean }) {
	const reduceMotion = useReducedMotion();
	const { rive, RiveComponent } = useRive({
		src: "/rive/penciljump.riv",
		autoplay: !reduceMotion
	});

	useEffect(() => {
		if (!rive) return;
		if (active && !reduceMotion) {
			rive.play();
		} else {
			rive.pause();
		}
	}, [rive, active, reduceMotion]);

	return (
		<Layer active={active}>
			<RiveComponent />
		</Layer>
	);
}
