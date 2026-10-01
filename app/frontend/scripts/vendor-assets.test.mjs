// node --test scripts/ (plain node: the build scripts are not app code, so
// they stay out of vitest's jsdom setup)
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkVendoredAssets, iconNamesIn, replaceDir, vendorAssets } from "./vendor-assets.mjs";

/** A throwaway frontend root with an ionicons install and a stale public/vendor/. */
function fakeFrontend(t, { withIonicons = true } = {}) {
	const dir = mkdtempSync(join(tmpdir(), "vendor-assets-test-"));
	t.after(() => rmSync(dir, { recursive: true, force: true }));
	mkdirSync(join(dir, "src"), { recursive: true });
	writeFileSync(join(dir, "src", "App.tsx"), "export const x = 1;\n");
	mkdirSync(join(dir, "public", "vendor"), { recursive: true });
	writeFileSync(join(dir, "public", "vendor", "previous.txt"), "old build");
	if (withIonicons) {
		const dist = join(dir, "node_modules", "ionicons", "dist");
		mkdirSync(join(dist, "ionicons"), { recursive: true });
		mkdirSync(join(dist, "svg"), { recursive: true });
		writeFileSync(join(dist, "ionicons", "ionicons.esm.js"), "// loader");
		writeFileSync(join(dist, "ionicons", "p-abc.js"), "// chunk");
		writeFileSync(join(dist, "svg", "add.svg"), "<svg/>");
	}
	return dir;
}

const stagingLeftovers = (dir) => readdirSync(dir).filter((f) => f.startsWith(".vendor-staging-"));

test("vendorAssets swaps a complete tree into public/vendor", (t) => {
	const dir = fakeFrontend(t);
	vendorAssets(dir);
	const vendor = join(dir, "public", "vendor");
	assert.ok(existsSync(join(vendor, "ionicons", "ionicons.esm.js")));
	assert.ok(existsSync(join(vendor, "ionicons", "svg", "add.svg")));
	assert.ok(existsSync(join(vendor, "rive", "rive.wasm")));
	assert.ok(!existsSync(join(vendor, "previous.txt")), "stale files are replaced");
	assert.deepEqual(stagingLeftovers(dir), []);
});

test("a failed run leaves the previous public/vendor untouched", (t) => {
	// node_modules without ionicons: vendoring throws part-way through
	const dir = fakeFrontend(t, { withIonicons: false });
	assert.throws(() => vendorAssets(dir), /ENOENT/);
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

test("check mode passes on a fresh vendor tree", (t) => {
	const dir = fakeFrontend(t);
	writeFileSync(join(dir, "src", "App.tsx"), '<ion-icon name="add" />\n');
	vendorAssets(dir);
	assert.deepEqual(checkVendoredAssets(dir), []);
});

test("check mode reports missing, stale and extra vendored files", (t) => {
	const dir = fakeFrontend(t);
	assert.deepEqual(checkVendoredAssets(dir), [
		"public/vendor missing: ionicons/ionicons.esm.js",
		"public/vendor missing: ionicons/p-abc.js",
		"public/vendor missing: ionicons/svg/add.svg",
		"public/vendor missing: rive/rive.wasm",
		"public/vendor missing: rive/rive_fallback.wasm",
		"public/vendor unexpected: previous.txt"
	]);
	vendorAssets(dir);
	writeFileSync(join(dir, "public", "vendor", "ionicons", "p-abc.js"), "// edited");
	assert.deepEqual(checkVendoredAssets(dir), ["public/vendor out of date: ionicons/p-abc.js"]);
	rmSync(join(dir, "public", "vendor"), { recursive: true });
	assert.deepEqual(checkVendoredAssets(dir), ["public/vendor is missing"]);
});

test("check mode fails on an icon name without an svg", (t) => {
	const dir = fakeFrontend(t);
	writeFileSync(join(dir, "src", "App.tsx"), '<Button icon="ad" />\n');
	vendorAssets(dir);
	assert.deepEqual(checkVendoredAssets(dir), ["unknown icon name: ad (src/App.tsx)"]);
});
