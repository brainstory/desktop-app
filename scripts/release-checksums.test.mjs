// node --test scripts/ (plain node: the build scripts are not app code, so
// they stay out of vitest's jsdom setup)
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
	existsSync,
	mkdirSync,
	mkdtempSync,
	readdirSync,
	readFileSync,
	rmSync,
	writeFileSync
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));

// the filename release assets are verified against (`sha256sum -c
// SHA256SUMS.txt` from the download directory), pinned here on purpose
const MANIFEST_NAME = "SHA256SUMS.txt";

// loaded lazily so a missing script fails each test individually instead of
// failing the whole file at import time
const loadChecksums = () => import("./release-checksums.mjs");

/** The full flat asset set the release job has after its flatten step. */
const REQUIRED_ASSETS = {
	"Brainstory_0.4.1_aarch64.app.tar.gz": "mac updater tar",
	"Brainstory_0.4.1_aarch64.app.tar.gz.sig": "mac updater signature",
	"Brainstory_0.4.1_aarch64.dmg": "mac installer",
	"Brainstory_0.4.1_x64-setup.exe": "windows updater + installer",
	"Brainstory_0.4.1_x64-setup.exe.sig": "windows updater signature",
	"Brainstory_0.4.1_amd64.AppImage": "linux updater image",
	"Brainstory_0.4.1_amd64.AppImage.sig": "linux updater signature",
	"Brainstory_0.4.1_amd64.deb": "linux debian installer",
	"Brainstory_0.4.1_amd64.rpm": "linux rpm installer",
	"latest.json": "{}\n"
};

function fakeArtifacts(t, { dmgName = "Brainstory_0.4.1_aarch64.dmg" } = {}) {
	const dir = mkdtempSync(join(tmpdir(), "release-checksums-test-"));
	t.after(() => rmSync(dir, { recursive: true, force: true }));
	for (const [name, content] of Object.entries(REQUIRED_ASSETS)) {
		writeFileSync(join(dir, name === "Brainstory_0.4.1_aarch64.dmg" ? dmgName : name), content);
	}
	return dir;
}

const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");

/**
 * What `sha256sum -c SHA256SUMS.txt` does from the directory users download
 * release assets into: each line is `<64 hex>  <relative name>`, and the file
 * is looked up next to the manifest.
 */
function verifyFromDownloadDir(dir) {
	const problems = [];
	for (const line of readFileSync(join(dir, MANIFEST_NAME), "utf8").split("\n")) {
		if (line === "") continue;
		const match = /^([0-9a-f]{64}) {2}(.+)$/.exec(line);
		if (!match) {
			problems.push(`unparseable manifest line: ${line}`);
			continue;
		}
		if (match[2].includes("/")) problems.push(`entry is not a relative basename: ${match[2]}`);
		else if (!existsSync(join(dir, match[2]))) problems.push(`no such file: ${match[2]}`);
		else if (sha256(join(dir, match[2])) !== match[1])
			problems.push(`digest mismatch: ${match[2]}`);
	}
	return problems;
}

/** The real thing, when the platform has the binary (release runners do). */
function sha256sumCheck(dir) {
	if (spawnSync("sha256sum", ["--version"]).status !== 0) return null;
	return spawnSync("sha256sum", ["-c", MANIFEST_NAME], { cwd: dir, encoding: "utf8" });
}

const manifestNames = (dir) =>
	readFileSync(join(dir, MANIFEST_NAME), "utf8")
		.split("\n")
		.filter(Boolean)
		.map((line) => line.slice(66));

test("first generation verifies from the download directory", async (t) => {
	const { generateChecksums } = await loadChecksums();
	const dir = fakeArtifacts(t);
	await generateChecksums(dir);
	assert.deepEqual(verifyFromDownloadDir(dir), []);
	const check = sha256sumCheck(dir);
	assert.ok(!check || check.status === 0, check?.stderr);
	// every asset, and only the assets, are listed
	assert.deepEqual(
		readdirSync(dir)
			.filter((n) => n !== MANIFEST_NAME)
			.sort(),
		[...manifestNames(dir)].sort()
	);
});

test("repeat generation verifies and stays byte-identical", async (t) => {
	const { generateChecksums } = await loadChecksums();
	const dir = fakeArtifacts(t);
	await generateChecksums(dir);
	const first = readFileSync(join(dir, MANIFEST_NAME), "utf8");
	await generateChecksums(dir);
	assert.equal(readFileSync(join(dir, MANIFEST_NAME), "utf8"), first);
	assert.deepEqual(verifyFromDownloadDir(dir), []);
	const check = sha256sumCheck(dir);
	assert.ok(!check || check.status === 0, check?.stderr);
});

test("the manifest never lists itself", async (t) => {
	const { generateChecksums } = await loadChecksums();
	const dir = fakeArtifacts(t);
	writeFileSync(join(dir, MANIFEST_NAME), "stale manifest from an earlier run");
	await generateChecksums(dir);
	const names = manifestNames(dir);
	assert.ok(!names.includes(MANIFEST_NAME));
	assert.equal(names.length, Object.keys(REQUIRED_ASSETS).length);
});

test("filenames with spaces verify from the download directory", async (t) => {
	const { generateChecksums } = await loadChecksums();
	const dir = fakeArtifacts(t, { dmgName: "Brainstory Desktop 0.4.1.dmg" });
	await generateChecksums(dir);
	assert.deepEqual(verifyFromDownloadDir(dir), []);
	const check = sha256sumCheck(dir);
	assert.ok(!check || check.status === 0, check?.stderr);
});

test("changing an asset updates its digest", async (t) => {
	const { generateChecksums } = await loadChecksums();
	const dir = fakeArtifacts(t);
	await generateChecksums(dir);
	const before = readFileSync(join(dir, MANIFEST_NAME), "utf8");
	writeFileSync(join(dir, "Brainstory_0.4.1_amd64.deb"), "tampered deb");
	await generateChecksums(dir);
	const after = readFileSync(join(dir, MANIFEST_NAME), "utf8");
	assert.notEqual(after, before);
	assert.deepEqual(verifyFromDownloadDir(dir), []);
	assert.notEqual(
		before.split("\n").find((line) => line.endsWith(".deb")),
		after.split("\n").find((line) => line.endsWith(".deb")),
		"the .deb line changed"
	);
});

test("a failed run leaves the previous manifest intact and publishes nothing partial", async (t) => {
	const { generateChecksums } = await loadChecksums();
	const dir = fakeArtifacts(t);
	await generateChecksums(dir);
	const good = readFileSync(join(dir, MANIFEST_NAME), "utf8");
	rmSync(join(dir, "latest.json"));
	await assert.rejects(generateChecksums(dir), /latest\.json/);
	assert.equal(
		readFileSync(join(dir, MANIFEST_NAME), "utf8"),
		good,
		"previous manifest untouched"
	);
	assert.deepEqual(
		readdirSync(dir).filter(
			(n) =>
				n !== MANIFEST_NAME &&
				!(n in REQUIRED_ASSETS) &&
				n !== "Brainstory Desktop 0.4.1.dmg"
		),
		[],
		"no partial manifest or temp file was left behind"
	);
});

test("unexpected directories fail the run loudly", async (t) => {
	const { generateChecksums } = await loadChecksums();
	const dir = fakeArtifacts(t);
	mkdirSync(join(dir, "deb"));
	await assert.rejects(generateChecksums(dir), /unexpected director/);
	assert.ok(!existsSync(join(dir, MANIFEST_NAME)), "nothing was published");
});

test("every missing required asset is named in the failure", async (t) => {
	const { generateChecksums } = await loadChecksums();
	for (const [remove, requirement] of [
		["latest.json", "latest.json"],
		["Brainstory_0.4.1_aarch64.app.tar.gz.sig", "*.app.tar.gz.sig"],
		["Brainstory_0.4.1_amd64.AppImage.sig", "*.AppImage.sig"],
		["Brainstory_0.4.1_x64-setup.exe", "*setup.exe"],
		["Brainstory_0.4.1_x64-setup.exe.sig", "*setup.exe.sig"],
		["Brainstory_0.4.1_aarch64.dmg", "*.dmg"],
		["Brainstory_0.4.1_amd64.deb", "*.deb"],
		["Brainstory_0.4.1_amd64.rpm", "*.rpm"]
	]) {
		const dir = fakeArtifacts(t);
		rmSync(join(dir, remove));
		const pattern = requirement.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
		await assert.rejects(
			generateChecksums(dir),
			new RegExp(pattern),
			`removing ${remove} must name ${requirement}`
		);
	}
});

test("the CLI command the workflow runs writes a verifying manifest", (t) => {
	const dir = fakeArtifacts(t);
	const run = spawnSync(process.execPath, [join(scriptDir, "release-checksums.mjs"), dir], {
		encoding: "utf8"
	});
	assert.equal(run.status, 0, run.stderr);
	assert.ok(existsSync(join(dir, MANIFEST_NAME)));
	const good = readFileSync(join(dir, MANIFEST_NAME), "utf8");
	assert.deepEqual(verifyFromDownloadDir(dir), []);

	rmSync(join(dir, "Brainstory_0.4.1_amd64.rpm"));
	const failed = spawnSync(process.execPath, [join(scriptDir, "release-checksums.mjs"), dir], {
		encoding: "utf8"
	});
	assert.notEqual(failed.status, 0, "a broken asset set exits non-zero");
	assert.match(failed.stderr, /\.rpm/);
	assert.equal(
		readFileSync(join(dir, MANIFEST_NAME), "utf8"),
		good,
		"the failed run published nothing"
	);
});
