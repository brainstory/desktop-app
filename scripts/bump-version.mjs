#!/usr/bin/env node
// Bump the app version everywhere it must agree, so a release tag can
// never diverge from the committed versions:
//   node scripts/bump-version.mjs 0.3.0
// Updates src-tauri/tauri.conf.json, src-tauri/Cargo.toml, the root
// package.json, and src-tauri/Cargo.lock.
import { readFileSync, writeFileSync } from "node:fs";
import { execSync } from "node:child_process";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
	console.error("usage: node scripts/bump-version.mjs <semver>   e.g. 0.3.0");
	process.exit(1);
}

/** Replace the first `"version": "..."` occurrence, preserving formatting. */
function setJsonVersion(path) {
	const text = readFileSync(path, "utf8");
	const updated = text.replace(/("version"\s*:\s*")[^"]*(")/, `$1${version}$2`);
	if (updated === text && !text.includes(`"version": "${version}"`)) {
		throw new Error(`no version field found in ${path}`);
	}
	writeFileSync(path, updated);
	console.log(`bumped ${path}`);
}

setJsonVersion("src-tauri/tauri.conf.json");
setJsonVersion("package.json");

// Cargo.toml: only the [package] version (the first version = line)
{
	const path = "src-tauri/Cargo.toml";
	const text = readFileSync(path, "utf8");
	const updated = text.replace(/^version\s*=\s*"[^"]*"/m, `version = "${version}"`);
	if (updated === text && !text.includes(`version = "${version}"`)) {
		throw new Error(`no version field found in ${path}`);
	}
	writeFileSync(path, updated);
	console.log(`bumped ${path}`);
}

// Cargo.lock follows the crate version
execSync(`cargo update -p brainstory --precise ${version}`, {
	cwd: "src-tauri",
	stdio: "inherit"
});
console.log(`bumped src-tauri/Cargo.lock`);
console.log(`\nall manifests are now ${version}; commit them and tag v${version}`);
