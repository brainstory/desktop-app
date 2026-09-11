// Vendors the external assets the app used to load from CDNs so it runs
// fully offline: the ionicons web-component loader + SVG icons, and the
// Rive WASM engine. Fonts are handled by @fontsource-variable/inter, which
// Vite bundles directly.
//
// Run automatically by `pnpm dev` / `pnpm build`.
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const root = dirname(dirname(fileURLToPath(import.meta.url)));
const outDir = join(root, "public", "vendor", "ionicons");

// The ionicons ESM loader resolves its chunks and icon SVGs relative to its
// own URL, so serving the whole set from public/vendor/ionicons/ keeps every
// runtime request local.
const ioniconsDir = join(root, "node_modules", "ionicons", "dist", "ionicons");
const svgDir = join(root, "node_modules", "ionicons", "dist", "svg");

rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });

// Loader chunks the ESM path can request (skip the systemjs/nomodule builds).
let loaders = 0;
for (const file of readdirSync(ioniconsDir)) {
	const isLoader =
		file === "ionicons.esm.js" ||
		(/^p-.*\.js$/.test(file) && !file.includes("system"));
	if (isLoader && statSync(join(ioniconsDir, file)).isFile()) {
		cpSync(join(ioniconsDir, file), join(outDir, file));
		loaders++;
	}
}

// All icon SVGs: names are used dynamically in several components (star
// ratings, save states, topic cards), so cherry-picking is fragile.
cpSync(svgDir, join(outDir, "svg"), { recursive: true, dereference: true });
const iconCount = readdirSync(join(outDir, "svg")).length;

// Typo guard: every literal icon name in the source must exist as an SVG.
const srcDir = join(root, "src");
const names = new Set();
const walk = (dir) => {
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		const path = join(dir, entry.name);
		if (entry.isDirectory()) walk(path);
		else if (/\.(jsx|js|astro)$/.test(entry.name)) {
			const text = readFileSync(path, "utf8");
			for (const match of text.matchAll(/(?:name|icon|iconName)\s*[:=]\s*"?([a-z0-9]+(?:-[a-z0-9]+)+)"?/g)) {
				names.add(match[1]);
			}
			for (const match of text.matchAll(/\?\s*"(?:[a-z0-9-]+)"\s*:\s*"([a-z0-9-]+(?:-[a-z0-9]+)+)"/g)) {
				names.add(match[1]);
			}
		}
	}
};
walk(srcDir);
const missing = [...names].filter(
	(name) => !existsSync(join(outDir, "svg", `${name}.svg`))
);
// Filter out values that are clearly not icon names (matched props of other
// components); only flag candidates that look like ionicon names.
const suspicious = missing.filter((name) =>
	/(outline|sharp|filled|-off|-circle|-up|-down|-back|-forward|chevron|star|mic|flash)$/.test(name)
);

console.log(
	`vendored ionicons: ${loaders} loader files, ${iconCount} svg icons -> public/vendor/ionicons`
);
if (suspicious.length) {
	console.warn(
		`warning: possible icon names without a matching svg: ${suspicious.join(", ")}`
	);
}

// ---- Rive WASM engine ----
// @rive-app/react-canvas fetches its WASM from unpkg by default; serve the
// copy shipped in the @rive-app/canvas package instead (see RivePencil.jsx).
const riveOut = join(root, "public", "vendor", "rive");
const riveCanvasDir = (() => {
	try {
		// resolve the transitive dependency from the react-canvas package
		const reactCanvas = dirname(require.resolve("@rive-app/react-canvas/package.json"));
		return dirname(require.resolve("@rive-app/canvas/package.json", { paths: [reactCanvas] }));
	} catch {
		return null;
	}
})();
if (riveCanvasDir) {
	rmSync(riveOut, { recursive: true, force: true });
	mkdirSync(riveOut, { recursive: true });
	for (const file of ["rive.wasm", "rive_fallback.wasm"]) {
		const src = join(riveCanvasDir, file);
		if (!existsSync(src)) {
			throw new Error(`rive wasm missing from package: ${src}`);
		}
		cpSync(src, join(riveOut, file));
	}
	console.log("vendored rive: rive.wasm + rive_fallback.wasm -> public/vendor/rive");
} else {
	throw new Error("could not resolve @rive-app/canvas - run pnpm install first");
}
