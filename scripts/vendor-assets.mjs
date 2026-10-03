// Vendors the external assets the app used to load from CDNs so it runs
// fully offline: the ionicons web-component loader + SVG icons, and the
// Rive WASM engine. Fonts are handled by @fontsource-variable/inter, which
// Vite bundles directly.
//
// Run automatically by `pnpm dev` / `pnpm build`. Everything is written to a
// staging directory first and swapped into public/vendor/ only once complete,
// so a failed run (e.g. node_modules missing a package) leaves the previous
// assets in place instead of an empty or half-filled public/vendor/.
//
// `--check` (pnpm vendor:check) writes nothing: it exits non-zero when
// public/vendor/ is missing or differs from what a fresh run would produce,
// or when the source names an ionicon that has no SVG.
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
import { join, dirname, relative, resolve } from "node:path";
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
	return `ionicons: ${loaders} loader files, ${iconCount} svg icons`;
}

/**
 * The expression assigned at `start`: a "literal", a balanced {...} JSX
 * expression, or a plain expression up to `;`, `,` or a closing bracket
 * (continuing across lines that start with `?` or `:`, i.e. a ternary).
 */
function valueAt(text, start) {
	if (text[start] === '"') return text.slice(start, text.indexOf('"', start + 1) + 1);
	let depth = 0;
	for (let i = start; i < text.length; i++) {
		const c = text[i];
		if (c === '"') {
			const end = text.indexOf('"', i + 1);
			if (end < 0) break;
			i = end;
		} else if ("([{".includes(c)) depth++;
		else if (")]}".includes(c)) {
			if (depth === 0) return text.slice(start, i);
			depth--;
			if (depth === 0 && text[start] === "{") return text.slice(start, i + 1);
		} else if (depth === 0 && (c === ";" || c === ",")) return text.slice(start, i);
		else if (depth === 0 && c === "\n" && !/^\s*[?:]/.test(text.slice(i + 1, i + 200))) {
			return text.slice(start, i);
		}
	}
	return text.slice(start);
}

/** String literals a value can evaluate to: the whole value or a ternary branch. */
function literalsOf(value, names) {
	const whole = /^\s*"([^"]*)"\s*$/.exec(value);
	if (whole) names.add(whole[1]);
	// only literals right after ? or : - in `getQueryParam("id") ? "x" : null`
	// the "id" is an argument, not an icon
	for (const branch of value.matchAll(/[?:]\s*"([^"]*)"/g)) names.add(branch[1]);
}

/**
 * Literal ionicon names one source file references: `name` on <ion-icon>,
 * and props/keys/variables named icon, iconName, *Icon or *IconName.
 */
export function iconNamesIn(text) {
	const names = new Set();
	for (const tag of text.matchAll(/<ion-icon\b[^>]*>/g)) {
		const attr = /\bname=/.exec(tag[0]);
		if (attr) literalsOf(valueAt(tag[0], attr.index + attr[0].length), names);
	}
	for (const m of text.matchAll(
		/\b(?:icon|iconName|\w+Icon|\w+IconName)\s*(?:=(?![=>])|:)\s*/g
	)) {
		literalsOf(valueAt(text, m.index + m[0].length), names);
	}
	names.delete("");
	return names;
}

/** Icon names used under `<frontendRoot>/src` that have no SVG in `svgDir`. */
function unknownIconNames(frontendRoot, svgDir) {
	const unknown = [];
	const walk = (dir) => {
		for (const entry of readdirSync(dir, { withFileTypes: true })) {
			const path = join(dir, entry.name);
			if (entry.isDirectory()) walk(path);
			else if (/\.(tsx?|jsx?|astro)$/.test(entry.name)) {
				for (const name of iconNamesIn(readFileSync(path, "utf8"))) {
					const exists =
						/^[a-z0-9-]+$/.test(name) && existsSync(join(svgDir, `${name}.svg`));
					if (!exists) unknown.push(`${name} (${relative(frontendRoot, path)})`);
				}
			}
		}
	};
	walk(join(frontendRoot, "src"));
	return unknown.sort();
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
	return "rive: rive.wasm + rive_fallback.wasm";
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

/** Relative paths of every file under `dir` (empty when it doesn't exist). */
function filesUnder(dir, base = dir, out = []) {
	if (!existsSync(dir)) return out;
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		const path = join(dir, entry.name);
		if (entry.isDirectory()) filesUnder(path, base, out);
		else out.push(relative(base, path));
	}
	return out;
}

/** How `actual` differs from `expected`, one line per file. */
function diffTrees(expected, actual) {
	// sorted: readdir order is filesystem-dependent (hash order on ext4)
	const want = filesUnder(expected).sort();
	const have = new Set(filesUnder(actual));
	const problems = [];
	for (const file of want) {
		if (!have.has(file)) problems.push(`missing: ${file}`);
		else if (!readFileSync(join(expected, file)).equals(readFileSync(join(actual, file)))) {
			problems.push(`out of date: ${file}`);
		}
		have.delete(file);
	}
	for (const file of [...have].sort()) problems.push(`unexpected: ${file}`);
	return problems;
}

/** Build every asset into a fresh staging dir; the caller removes it. */
function buildStaging(frontendRoot) {
	// staging lives in the frontend root, not public/, so a crash can never
	// leave a half-written copy where astro would serve or bundle it
	const staging = mkdtempSync(join(frontendRoot, ".vendor-staging-"));
	try {
		const summary = [
			vendorIonicons(frontendRoot, join(staging, "ionicons")),
			vendorRive(join(staging, "rive"))
		];
		const unknown = unknownIconNames(frontendRoot, join(staging, "ionicons", "svg"));
		return { staging, summary, unknown };
	} catch (err) {
		rmSync(staging, { recursive: true, force: true });
		throw err;
	}
}

/** Vendor every asset into `<frontendRoot>/public/vendor/`, all or nothing. */
export function vendorAssets(frontendRoot = root) {
	const { staging, summary, unknown } = buildStaging(frontendRoot);
	try {
		replaceDir(staging, join(frontendRoot, "public", "vendor"));
	} finally {
		rmSync(staging, { recursive: true, force: true });
	}
	for (const line of summary) console.log(`vendored ${line} -> public/vendor`);
	if (unknown.length) {
		console.warn(`warning: icon names without a matching svg: ${unknown.join(", ")}`);
	}
}

/**
 * Verify without writing: public/vendor/ must match a fresh build exactly
 * and every icon name in the source must exist. Returns the problems found.
 */
export function checkVendoredAssets(frontendRoot = root) {
	const { staging, unknown } = buildStaging(frontendRoot);
	try {
		const vendorDir = join(frontendRoot, "public", "vendor");
		const problems = existsSync(vendorDir)
			? diffTrees(staging, vendorDir).map((p) => `public/vendor ${p}`)
			: ["public/vendor is missing"];
		return [...problems, ...unknown.map((name) => `unknown icon name: ${name}`)];
	} finally {
		rmSync(staging, { recursive: true, force: true });
	}
}

// importable (for tests) without vendoring anything
if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
	if (process.argv.includes("--check")) {
		const problems = checkVendoredAssets();
		for (const problem of problems) console.error(problem);
		if (problems.length) {
			console.error(`vendor check failed (${problems.length}); run: pnpm vendor`);
			process.exitCode = 1;
		} else {
			console.log("vendored assets are present and up to date; all icon names resolve");
		}
	} else {
		vendorAssets();
	}
}
