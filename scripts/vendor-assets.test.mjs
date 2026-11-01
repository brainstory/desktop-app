// node --test scripts/ (plain node: the build scripts are not app code, so
// they stay out of vitest's jsdom setup)
import { test } from "node:test";
import assert from "node:assert/strict";
import {
	existsSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	readdirSync,
	rmSync,
	writeFileSync
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
	checkVendoredAssets,
	iconNamesFromIconModule,
	iconNamesIn,
	replaceDir,
	vendorAssets
} from "./vendor-assets.mjs";

/** The icon module fixture: the typed list is the sprite's source. */
function writeIconModule(dir, names) {
	const iconDir = join(dir, "src", "components", "global", "design-system");
	mkdirSync(iconDir, { recursive: true });
	writeFileSync(
		join(iconDir, "Icon.tsx"),
		`export const ICON_NAMES = [\n${names.map((n) => `\t"${n}"`).join(",\n")}\n] as const;\n`
	);
}

/** A throwaway frontend root with an ionicons svg dir and a stale public/vendor/. */
function fakeFrontend(t, { withIonicons = true } = {}) {
	const dir = mkdtempSync(join(tmpdir(), "vendor-assets-test-"));
	t.after(() => rmSync(dir, { recursive: true, force: true }));
	mkdirSync(join(dir, "src"), { recursive: true });
	writeFileSync(join(dir, "src", "App.tsx"), "export const x = 1;\n");
	writeIconModule(dir, ["add", "square"]);
	mkdirSync(join(dir, "public", "vendor"), { recursive: true });
	writeFileSync(join(dir, "public", "vendor", "previous.txt"), "old build");
	if (withIonicons) {
		const svgDir = join(dir, "node_modules", "ionicons", "dist", "svg");
		mkdirSync(svgDir, { recursive: true });
		writeFileSync(
			join(svgDir, "add.svg"),
			'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512"><path d="M1 1"/></svg>'
		);
		writeFileSync(
			join(svgDir, "square.svg"),
			'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M2 2"/></svg>'
		);
	}
	return dir;
}

const stagingLeftovers = (dir) => readdirSync(dir).filter((f) => f.startsWith(".vendor-staging-"));

test("vendorAssets swaps a complete tree into public/vendor", (t) => {
	const dir = fakeFrontend(t);
	writeFileSync(join(dir, "src", "App.tsx"), '<Icon name="add" />\n');
	vendorAssets(dir);
	const vendor = join(dir, "public", "vendor");
	assert.ok(existsSync(join(vendor, "rive", "rive.wasm")));
	assert.ok(
		!existsSync(join(vendor, "ionicons")),
		"only the sprite is built from ionicons; the web-component set is not vendored"
	);
	assert.ok(!existsSync(join(vendor, "previous.txt")), "stale files are replaced");
	const sprite = readFileSync(join(dir, "public", "icons.svg"), "utf8");
	assert.ok(sprite.includes('<symbol id="add"'), "the sprite carries the used icon");
	// each symbol keeps its own SVG's viewBox, not a hard-coded one
	assert.ok(
		sprite.includes('<symbol id="add" viewBox="0 0 448 512">'),
		`the add symbol carries its source viewBox: ${sprite}`
	);
	assert.deepEqual(stagingLeftovers(dir), []);
});

test("a failed run leaves the previous public/vendor untouched", (t) => {
	// node_modules without ionicons: the sprite build throws part-way through
	const dir = fakeFrontend(t, { withIonicons: false });
	assert.throws(() => vendorAssets(dir), /ionicons is not installed/);
	assert.ok(!existsSync(join(dir, "public", "icons.svg")), "nothing is published");
	assert.ok(existsSync(join(dir, "public", "vendor", "previous.txt")));
	assert.deepEqual(stagingLeftovers(dir), []);
});

test("replaceDir moves src into a missing dest", (t) => {
	const dir = mkdtempSync(join(tmpdir(), "vendor-assets-test-"));
	t.after(() => rmSync(dir, { recursive: true, force: true }));
	mkdirSync(join(dir, "src"));
	writeFileSync(join(dir, "src", "a.txt"), "a");
	replaceDir(join(dir, "src"), join(dir, "nested", "dest"));
	assert.ok(existsSync(join(dir, "nested", "dest", "a.txt")));
	assert.ok(!existsSync(join(dir, "src")));
});

test("replaceDir restores dest when the swap fails", (t) => {
	const dir = mkdtempSync(join(tmpdir(), "vendor-assets-test-"));
	t.after(() => rmSync(dir, { recursive: true, force: true }));
	mkdirSync(join(dir, "dest"));
	writeFileSync(join(dir, "dest", "keep.txt"), "keep");
	assert.throws(() => replaceDir(join(dir, "does-not-exist"), join(dir, "dest")));
	assert.ok(existsSync(join(dir, "dest", "keep.txt")));
	assert.deepEqual(readdirSync(dir), ["dest"]);
});

test("iconNamesIn finds literal icon names, not other strings", () => {
	const source = `
		const NAV = [{ icon: "add", text: "New-item", href: "/chat" }];
		type Props = { iconName: string; leftButtonIcon?: string | null };
		const saveIconName =
			saveState === SAVING
				? "sync-outline" //reload-circle-outline
				: saveState === FAILED
					? "close-outline"
					: null;
		<ion-icon name="square" class="w-8 h-8 md hydrated" />
		<ion-icon
			class="w-4 h-4 hydrated"
			name="mic-off"
		></ion-icon>
		<TransparentButton classes="px-2 justify-center" icon="close" sr="Close modal" />
		<EndChatButton icon={isFinishing ? null : "exit-outline"} />
		<ChatTopBar
			leftButtonIcon={
				fromGuide && !getQueryParam("id") ? "arrow-back-outline" : null
			}
		/>
		<meta name="theme-color" content="#fff" />
	`;
	assert.deepEqual([...iconNamesIn(source)].sort(), [
		"add",
		"arrow-back-outline",
		"close",
		"close-outline",
		"exit-outline",
		"mic-off",
		"square",
		"sync-outline"
	]);
});

test("iconNamesFromIconModule reads the typed list", () => {
	const dir = mkdtempSync(join(tmpdir(), "vendor-assets-test-"));
	rmSync(dir, { recursive: true, force: true });
	writeIconModule(dir, ["add", "mic-off"]);
	assert.deepEqual(iconNamesFromIconModule(dir), ["add", "mic-off"]);
	rmSync(dir, { recursive: true, force: true });
});

test("check mode passes on a fresh vendor tree", (t) => {
	const dir = fakeFrontend(t);
	writeFileSync(join(dir, "src", "App.tsx"), '<ion-icon name="add" />\n');
	vendorAssets(dir);
	assert.deepEqual(checkVendoredAssets(dir), []);
});

test("check mode reports missing, stale and extra vendored files", (t) => {
	const dir = fakeFrontend(t);
	assert.deepEqual(checkVendoredAssets(dir), [
		"public/icons.svg is missing",
		"public/vendor missing: rive/rive.wasm",
		"public/vendor missing: rive/rive_fallback.wasm",
		"public/vendor unexpected: previous.txt"
	]);
	vendorAssets(dir);
	writeFileSync(join(dir, "public", "vendor", "rive", "rive.wasm"), "// edited");
	assert.deepEqual(checkVendoredAssets(dir), ["public/vendor out of date: rive/rive.wasm"]);
	rmSync(join(dir, "public", "vendor"), { recursive: true });
	assert.deepEqual(checkVendoredAssets(dir), ["public/vendor is missing"]);
});

test("check mode fails on an icon name outside the typed list", (t) => {
	const dir = fakeFrontend(t);
	writeFileSync(join(dir, "src", "App.tsx"), '<Button icon="ad" />\n');
	vendorAssets(dir);
	// the name is not in ICON_NAMES (and has no svg either): the fix is
	// to declare it, so the message says so
	assert.deepEqual(checkVendoredAssets(dir), [
		"unknown icon name: ad (src/App.tsx) - add it to ICON_NAMES"
	]);
});

test("check mode fails on a declared icon without an svg", (t) => {
	const dir = fakeFrontend(t);
	// declared in the list, but ionicons ships no such svg
	writeIconModule(dir, ["add", "square", "ghost-outline"]);
	writeFileSync(join(dir, "src", "App.tsx"), '<Icon name="ghost-outline" />\n');
	vendorAssets(dir);
	assert.deepEqual(checkVendoredAssets(dir), ["unknown icon name: ghost-outline (src/App.tsx)"]);
});
