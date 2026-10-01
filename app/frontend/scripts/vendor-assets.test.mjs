// node --test scripts/ (plain node: the build scripts are not app code, so
// they stay out of vitest's jsdom setup)
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { replaceDir, vendorAssets } from "./vendor-assets.mjs";

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
