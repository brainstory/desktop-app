// Generates the SHA256SUMS.txt manifest for a release, replacing the old
// inline `sha256sum artifacts/* > artifacts/SHA256SUMS.txt` pipeline, whose
// entries carried an artifacts/ prefix (so `sha256sum -c SHA256SUMS.txt`
// failed from the directory users download release assets into) and which
// hashed the truncated manifest into itself on a repeat run.
//
// Interface: node scripts/release-checksums.mjs <artifact-dir>
// (from the release job's workspace root: `node
// scripts/release-checksums.mjs artifacts`). The artifact
// directory must be flat - the workflow's flatten step guarantees that - and
// contain the full required asset set (installers, updater artifacts,
// signatures, latest.json); anything else fails the run loudly.
//
// The manifest lists relative basenames only, sorted, so verification works
// from any directory containing the downloaded assets:
//   sha256sum -c SHA256SUMS.txt
// The manifest itself is never an entry, so regenerating is byte-identical.
// All digests are computed before anything is written; the manifest is
// written to a temporary file in the artifact directory and atomically
// renamed, so a failure can never publish a partial manifest.
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { existsSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const MANIFEST_NAME = "SHA256SUMS.txt";
const TEMP_NAME = `.${MANIFEST_NAME}.tmp`;

// what a complete release must contain: installers, updater artifacts and
// their signatures (the set the updater-feed step consumes), plus the feed
// itself, which is generated before this step runs
const REQUIRED_EXACT = ["latest.json"];
const REQUIRED_SUFFIXES = [
	".app.tar.gz",
	".app.tar.gz.sig",
	".dmg",
	"setup.exe",
	"setup.exe.sig",
	".AppImage",
	".AppImage.sig",
	".deb",
	".rpm"
];

/** sha256 of one file, streamed so installer-sized assets stay off the heap. */
async function sha256File(path) {
	const hash = createHash("sha256");
	for await (const chunk of createReadStream(path)) hash.update(chunk);
	return hash.digest("hex");
}

/**
 * Validate the artifact directory and generate <dir>/SHA256SUMS.txt.
 * Returns the manifest text; throws (publishing nothing) when the directory
 * has subdirectories, non-file entries or is missing a required asset.
 */
export async function generateChecksums(dir) {
	const problems = [];
	const files = [];
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		if (entry.isDirectory()) {
			problems.push(
				`unexpected directory: ${entry.name} ` +
					`(the flatten step must leave a flat artifact tree)`
			);
		} else if (entry.isFile()) {
			if (entry.name !== MANIFEST_NAME && entry.name !== TEMP_NAME) files.push(entry.name);
		} else {
			problems.push(`unexpected non-file entry: ${entry.name}`);
		}
	}

	for (const name of REQUIRED_EXACT) {
		if (!files.includes(name)) problems.push(`missing required asset: ${name}`);
	}
	for (const suffix of REQUIRED_SUFFIXES) {
		if (!files.some((name) => name.endsWith(suffix))) {
			problems.push(`missing required asset matching '*${suffix}'`);
		}
	}
	if (problems.length) {
		throw new Error(
			`release-checksums: ${dir} is not a complete release:\n  ${problems.join("\n  ")}`
		);
	}

	// sorted: readdir order is filesystem-dependent (hash order on ext4)
	files.sort();
	const lines = [];
	for (const name of files) {
		lines.push(`${await sha256File(join(dir, name))}  ${name}`);
	}
	const manifest = `${lines.join("\n")}\n`;

	// write the complete manifest to a sibling temp file, then atomically
	// rename it into place; a failure anywhere above leaves any previous
	// manifest untouched and the finally below removes the temp
	const temp = join(dir, TEMP_NAME);
	const target = join(dir, MANIFEST_NAME);
	try {
		writeFileSync(temp, manifest);
		renameSync(temp, target);
	} finally {
		if (existsSync(temp)) rmSync(temp, { force: true });
	}
	return manifest;
}

// importable (for tests) without generating anything
if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
	const dir = process.argv[2];
	if (!dir) {
		console.error("usage: node scripts/release-checksums.mjs <artifact-dir>");
		process.exitCode = 2;
	} else {
		generateChecksums(dir).then(
			(manifest) => process.stdout.write(manifest),
			(err) => {
				console.error(err.message);
				process.exitCode = 1;
			}
		);
	}
}
