// Vendors the external assets the app used to load from CDNs so it runs
// fully offline: the ionicons web-component loader + SVG icons, and the
// Rive WASM engine. Fonts are handled by @fontsource-variable/inter, which
// Vite bundles directly.
//
// Run automatically by `pnpm dev` / `pnpm build`. Everything is written to a
// staging directory first and swapped into public/vendor/ only once complete,
// so a failed run (e.g. node_modules missing a package) leaves the previous
// assets in place instead of an empty or half-filled public/vendor/.
import {
	cpSync,
	existsSync,
	mkdirSync,
	mkdtempSync,
	readdirSync,
	readFileSync,
	renameSync,
	rmSync,
	statSync
} from "node:fs";
import { createRequire } from "node:module";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const require = createRequire(import.meta.url);
const root = dirname(dirname(fileURLToPath(import.meta.url)));

/** Copy the ionicons loader chunks + every SVG into `outDir`. */
function vendorIonicons(frontendRoot, outDir) {
	// The ionicons ESM loader resolves its chunks and icon SVGs relative to
	// its own URL, so serving the whole set from public/vendor/ionicons/
	// keeps every runtime request local.
	const ioniconsDir = join(frontendRoot, "node_modules", "ionicons", "dist", "ionicons");
	const svgDir = join(frontendRoot, "node_modules", "ionicons", "dist", "svg");
	mkdirSync(outDir, { recursive: true });

	// Loader chunks the ESM path can request (skip the systemjs/nomodule builds).
	let loaders = 0;
	for (const file of readdirSync(ioniconsDir)) {
		const isLoader =
			file === "ionicons.esm.js" || (/^p-.*\.js$/.test(file) && !file.includes("system"));
		if (isLoader && statSync(join(ioniconsDir, file)).isFile()) {
			cpSync(join(ioniconsDir, file), join(outDir, file));
			loaders++;
		}
	}

	// All icon SVGs: names are used dynamically in several components (star
	// ratings, save states, topic cards), so cherry-picking is fragile.
	cpSync(svgDir, join(outDir, "svg"), { recursive: true, dereference: true });
	const iconCount = readdirSync(join(outDir, "svg")).length;

	console.log(
		`vendored ionicons: ${loaders} loader files, ${iconCount} svg icons -> public/vendor/ionicons`
	);
}

/** Typo guard: literal icon names in the source that have no matching SVG. */
function suspiciousIconNames(frontendRoot, outDir) {
	const srcDir = join(frontendRoot, "src");
	const names = new Set();
	const walk = (dir) => {
		for (const entry of readdirSync(dir, { withFileTypes: true })) {
			const path = join(dir, entry.name);
			if (entry.isDirectory()) walk(path);
			else if (/\.(tsx?|jsx?|astro)$/.test(entry.name)) {
				const text = readFileSync(path, "utf8");
				for (const match of text.matchAll(
					/(?:name|icon|iconName)\s*[:=]\s*"?([a-z0-9]+(?:-[a-z0-9]+)+)"?/g
				)) {
					names.add(match[1]);
				}
				for (const match of text.matchAll(
					/\?\s*"(?:[a-z0-9-]+)"\s*:\s*"([a-z0-9-]+(?:-[a-z0-9]+)+)"/g
				)) {
					names.add(match[1]);
				}
			}
		}
	};
	walk(srcDir);
	const missing = [...names].filter((name) => !existsSync(join(outDir, "svg", `${name}.svg`)));
	// Filter out values that are clearly not icon names (matched props of other
	// components); only flag candidates that look like ionicon names.
	return missing.filter((name) =>
		/(outline|sharp|filled|-off|-circle|-up|-down|-back|-forward|chevron|star|mic|flash)$/.test(
			name
		)
	);
}

// ---- Rive WASM engine ----
// @rive-app/react-canvas fetches its WASM from unpkg by default; serve the
// copy shipped in the @rive-app/canvas package instead (see RivePencil.jsx).
function vendorRive(riveOut) {
	const riveCanvasDir = (() => {
		try {
			// resolve the transitive dependency from the react-canvas package
			const reactCanvas = dirname(require.resolve("@rive-app/react-canvas/package.json"));
			return dirname(
				require.resolve("@rive-app/canvas/package.json", { paths: [reactCanvas] })
			);
		} catch {
			return null;
		}
	})();
	if (!riveCanvasDir) {
		throw new Error("could not resolve @rive-app/canvas - run pnpm install first");
	}
	mkdirSync(riveOut, { recursive: true });
	for (const file of ["rive.wasm", "rive_fallback.wasm"]) {
		const src = join(riveCanvasDir, file);
		if (!existsSync(src)) {
			throw new Error(`rive wasm missing from package: ${src}`);
		}
		cpSync(src, join(riveOut, file));
	}
	console.log("vendored rive: rive.wasm + rive_fallback.wasm -> public/vendor/rive");
}

/**
 * Put the complete directory `src` at `dest`, replacing whatever was there.
 * Both renames stay on one filesystem (src is created next to the frontend
 * root), and a failed swap puts the previous `dest` back.
 */
export function replaceDir(src, dest) {
	const old = `${src}.old`;
	rmSync(old, { recursive: true, force: true });
	const hadDest = existsSync(dest);
	if (hadDest) renameSync(dest, old);
	try {
		mkdirSync(dirname(dest), { recursive: true });
		renameSync(src, dest);
	} catch (err) {
		if (hadDest) renameSync(old, dest);
		throw err;
	}
	rmSync(old, { recursive: true, force: true });
}

/** Vendor every asset into `<frontendRoot>/public/vendor/`, all or nothing. */
export function vendorAssets(frontendRoot = root) {
	// staging lives in the frontend root, not public/, so a crash can never
	// leave a half-written copy where astro would serve or bundle it
	const staging = mkdtempSync(join(frontendRoot, ".vendor-staging-"));
	try {
		const ioniconsOut = join(staging, "ionicons");
		vendorIonicons(frontendRoot, ioniconsOut);
		const suspicious = suspiciousIconNames(frontendRoot, ioniconsOut);
		if (suspicious.length) {
			console.warn(
				`warning: possible icon names without a matching svg: ${suspicious.join(", ")}`
			);
		}
		vendorRive(join(staging, "rive"));
		replaceDir(staging, join(frontendRoot, "public", "vendor"));
	} finally {
		rmSync(staging, { recursive: true, force: true });
	}
}

// importable (for tests) without vendoring anything
if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
	vendorAssets();
}
