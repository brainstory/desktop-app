// node --test scripts/ (plain node: the build scripts are not app code, so
// they stay out of vitest's jsdom setup)
//
// Task F20: the updater signing key must only ever be visible to the single
// explicit "Sign updater artifacts" step in the release job - never to
// `pnpm tauri build` (frontend hooks, Cargo build scripts and pnpm's
// auto-install all inherit the build step's environment). Two groups:
//
// 1. structural tests over .github/workflows/release.yml, parsed with a
//    minimal indentation-based reader (no YAML dependency exists in the
//    plain-node script layer and none may be added);
// 2. signature tests: the real tauri CLI signs fixtures with throwaway keys
//    into a temp dir (skipped when node_modules is absent), and the
//    minisign verification logic runs offline against synthetic signatures.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
	createHash,
	createPublicKey,
	generateKeyPairSync,
	sign as edSign,
	verify as edVerify
} from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../../..");
const workflowPath = join(repoRoot, ".github/workflows/release.yml");
const tauriCli = join(repoRoot, "node_modules/.bin/tauri");

// ---------------------------------------------------------------------------
// minimal indentation-based reader for the workflow's structure. Only what
// the structural assertions need: which steps exist in which job, their
// names, their env keys and their run text.
// ---------------------------------------------------------------------------

/** matches `key: value` / `key:` lines (not block-scalar content) */
const KEY_LINE = /^([A-Za-z_][\w.-]*):( |$)/;

function parseTree(text) {
	const root = { key: null, value: null, isItem: false, indent: -1, children: [], raw: "" };
	const stack = [root];
	for (const raw of text.split("\n")) {
		if (raw.trim() === "" || raw.trimStart().startsWith("#")) continue;
		const indent = raw.length - raw.trimStart().length;
		const trimmed = raw.trim();
		const isItem = trimmed.startsWith("- ");
		const content = isItem ? trimmed.slice(2) : trimmed;
		const node = { key: null, value: null, isItem, indent, children: [], raw };
		const match = KEY_LINE.exec(content);
		if (match) {
			node.key = match[1];
			node.value = content.slice(match[1].length + 1).trim();
		} else {
			node.value = content;
		}
		while (stack.length > 1 && stack[stack.length - 1].indent >= indent) stack.pop();
		stack[stack.length - 1].children.push(node);
		stack.push(node);
	}
	return root;
}

/** mapping of a step node's keys to nodes (an item's own `key: value` is a prop) */
function props(node) {
	const map = new Map();
	if (node.key !== null) map.set(node.key, node);
	for (const child of node.children) if (child.key !== null) map.set(child.key, child);
	return map;
}

/** the full text of a `run:` (or `path:`) block, inline value plus block lines */
function blockText(node) {
	if (!node) return "";
	const lines = node.value && node.value !== "|" ? [node.value] : [];
	for (const child of node.children) {
		lines.push(child.raw.trim());
		// a `run: |` body has no nested keys; cover any future nesting anyway
		for (const nested of child.children) lines.push(nested.raw.trim());
	}
	return lines.join("\n");
}

/** every node in the tree keyed `env`, anywhere (job level, step level, ...) */
function envNodes(node, found = []) {
	if (node.key === "env") found.push(node);
	for (const child of node.children) envNodes(child, found);
	return found;
}

function envKeyNode(envNode, name) {
	for (const child of envNode.children) if (child.key === name) return child;
	return null;
}

function readWorkflow() {
	const tree = parseTree(readFileSync(workflowPath, "utf8"));
	const jobs = new Map();
	const jobsNode = tree.children.find((c) => c.key === "jobs");
	for (const job of jobsNode?.children ?? []) {
		const stepsNode = job.children.find((c) => c.key === "steps");
		const stepList = [];
		for (const step of stepsNode?.children ?? []) {
			const p = props(step);
			stepList.push({
				name: p.get("name")?.value ?? "(unnamed)",
				run: blockText(p.get("run")),
				with: blockText(p.get("with")),
				env: p.get("env") ?? null
			});
		}
		// a mapping node is a step's own env only when nested in that step;
		// job-level env lives directly under the job node
		const jobEnvChildren = job.children.filter((c) => c.key === "env");
		jobs.set(job.key, { steps: stepList, jobEnv: jobEnvChildren });
	}
	return { tree, jobs };
}

const SIGNING_ENV = /^TAURI_SIGNING/;

/** every env node (at any level) holding updater-signing variables */
function signingEnvOwners(tree) {
	return envNodes(tree).filter((node) =>
		node.children.some((c) => SIGNING_ENV.test(c.key ?? ""))
	);
}

// ---------------------------------------------------------------------------
// structural tests (release.yml)
// ---------------------------------------------------------------------------

test("updater signing env exists only on the release job's explicit sign step", () => {
	const { tree, jobs } = readWorkflow();
	const owners = signingEnvOwners(tree);
	const described = owners.map((node) =>
		node.children
			.filter((c) => SIGNING_ENV.test(c.key ?? ""))
			.map((c) => c.key)
			.join(", ")
	);
	assert.equal(
		owners.length,
		1,
		`expected exactly one env block with TAURI_SIGNING variables, found ${owners.length} [${described.join("] [")}]`
	);

	// the owning env block must be a step property of the release job
	const release = jobs.get("release");
	assert.ok(release, "release job exists");
	const signSteps = release.steps.filter((step) => step.env === owners[0]);
	assert.equal(signSteps.length, 1, "the signing env belongs to exactly one release-job step");
	const signStep = signSteps[0];
	assert.equal(signStep.name, "Sign updater artifacts");

	const keys = owners[0].children.map((c) => c.key).sort();
	assert.deepEqual(keys, ["TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"]);
	assert.match(
		blockText(envKeyNode(owners[0], "TAURI_SIGNING_PRIVATE_KEY")),
		/secrets\.TAURI_SIGNING_PRIVATE_KEY/
	);
	assert.match(
		blockText(envKeyNode(owners[0], "TAURI_SIGNING_PRIVATE_KEY_PASSWORD")),
		/secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD/
	);
});

test("all three platform builds run without signing credentials and skip updater signing", () => {
	const { jobs } = readWorkflow();
	const build = jobs.get("build");
	assert.ok(build, "build job exists");
	for (const platform of ["macOS", "Windows", "Linux"]) {
		const steps = build.steps.filter((s) => s.name === `Build (${platform})`);
		assert.equal(steps.length, 1, `exactly one Build (${platform}) step`);
		const step = steps[0];
		const leaked =
			step.env?.children.map((c) => c.key).filter((k) => SIGNING_ENV.test(k)) ?? [];
		assert.deepEqual(
			leaked,
			[],
			`Build (${platform}) must not see the updater signing env (frontend hooks, Cargo build scripts and pnpm auto-install all inherit it)`
		);
		assert.match(step.run, /tauri build/, `Build (${platform}) builds via the tauri CLI`);
		assert.match(
			step.run,
			/--no-sign/,
			`Build (${platform}) passes --no-sign: without the key the bundler would fail demanding TAURI_SIGNING_PRIVATE_KEY instead of shipping unsigned artifacts`
		);
	}
});

test("the sign step invokes the installed tauri binary directly, never a package manager", () => {
	const { jobs } = readWorkflow();
	const signStep = jobs.get("release")?.steps.find((s) => s.name === "Sign updater artifacts");
	assert.ok(signStep, "sign step exists");
	assert.match(
		signStep.run,
		/node_modules\/\.bin\/tauri[^\n]*signer sign/,
		"signing runs the installed CLI binary directly"
	);
	assert.doesNotMatch(
		signStep.run,
		/\b(pnpm|npm|npx|yarn|corepack|cargo)\b/,
		"the step holding the key must not invoke a package manager or Cargo (pnpm run/exec auto-installs, and install scripts would inherit the key)"
	);
	assert.doesNotMatch(signStep.run, /\binstall\b/, "the sign step installs nothing");
});

test("release job signs after flatten, and before the updater feed and checksums", () => {
	const { jobs } = readWorkflow();
	const steps = jobs.get("release")?.steps ?? [];
	const names = steps.map((s) => s.name);
	const index = (fragment) => names.findIndex((n) => n.includes(fragment));
	const flatten = index("Flatten downloaded packages");
	const install = index("Install Tauri CLI");
	const sign = index("Sign updater artifacts");
	const feed = index("Generate updater feed");
	const checksums = index("Generate checksums");
	for (const [label, at] of [
		["Flatten downloaded packages", flatten],
		["Install Tauri CLI", install],
		["Sign updater artifacts", sign],
		["Generate updater feed", feed],
		["Generate checksums", checksums]
	]) {
		assert.ok(at >= 0, `${label} step exists in the release job`);
	}
	assert.ok(flatten < sign, "signing runs on the flattened artifact set");
	assert.ok(install < sign, "the CLI is installed (keyless) before signing");
	assert.ok(sign < feed, "the feed step consumes the .sig files the sign step produces");
	assert.ok(sign < checksums, "release-checksums.mjs requires the .sig files to exist");
});

test("build jobs upload unsigned updater artifacts; .sig sidecars come from the release job", () => {
	const { jobs } = readWorkflow();
	for (const platform of ["macOS", "Windows", "Linux"]) {
		const steps = jobs
			.get("build")
			.steps.filter((s) => s.name === `Upload updater artifacts (${platform})`);
		assert.equal(steps.length, 1, `exactly one Upload updater artifacts (${platform}) step`);
	}
	// nothing in the build job may reference .sig outputs anymore (they are
	// produced in the release job now), while the feed step must keep
	// consuming them
	const buildText = JSON.stringify(
		jobs.get("build").steps.map((s) => ({ name: s.name, run: s.run, with: s.with }))
	);
	assert.doesNotMatch(
		buildText,
		/\.sig/,
		"the build job neither produces nor uploads signatures (upload-artifact runs with if-no-files-found: error)"
	);
	const feed = jobs.get("release").steps.find((s) => s.name.includes("Generate updater feed"));
	assert.match(feed.run, /\.sig/, "the updater feed still reads the .sig sidecars");
});

// ---------------------------------------------------------------------------
// minisign verification helpers (mirrors rust-minisign's construction, which
// the tauri CLI uses: pure Ed25519 over BLAKE2b-512(file), keynum-prefixed
// boxes, plus a global signature over sig||trustedComment)
// ---------------------------------------------------------------------------

const b64 = (buf) => Buffer.from(buf).toString("base64");
const unb64 = (s) => Buffer.from(s.trim(), "base64");

/** the .pub file / updater pubkey is base64 of "comment\n<b64 box>\n" */
function parsePublicKeyBox(base64Text) {
	const box = unb64(base64Text).toString("utf8");
	const line = box.split("\n").find((l) => l.startsWith("RW"));
	assert.ok(line, "public key box has a key line");
	const raw = unb64(line); // sigalg(2) + keynum(8) + pk(32)
	assert.equal(raw.length, 42, "public key box is 42 bytes");
	return {
		alg: raw.subarray(0, 2).toString("latin1"),
		keynum: raw.subarray(2, 10),
		pk: raw.subarray(10)
	};
}

/** the .sig file is base64 of "untrusted comment\n<b64 sig>\ntrusted comment\n<b64 global>" */
function parseSignatureBox(base64Text) {
	const box = unb64(base64Text).toString("utf8");
	const lines = box.split("\n").filter(Boolean);
	assert.equal(lines.length, 4, "signature box has four lines");
	assert.match(lines[0], /^untrusted comment: /);
	assert.match(lines[2], /^trusted comment: /);
	const raw = unb64(lines[1]); // sigalg(2) + keynum(8) + sig(64)
	assert.equal(raw.length, 74, "signature box is 74 bytes");
	return {
		alg: raw.subarray(0, 2).toString("latin1"),
		keynum: raw.subarray(2, 10),
		sig: raw.subarray(10),
		trustedComment: lines[2].replace(/^trusted comment: /, ""),
		globalSig: unb64(lines[3])
	};
}

/** verify a .sig (base64 text) over fileBytes with a .pub (base64 text) */
function verifyUpdaterSignature(sigText, pubText, fileBytes) {
	const pk = parsePublicKeyBox(pubText);
	const sig = parseSignatureBox(sigText);
	assert.equal(pk.alg, "Ed", "public key algorithm");
	assert.equal(sig.alg, "ED", "signature algorithm (prehashed minisign)");
	assert.ok(pk.keynum.equals(sig.keynum), "signature keynum matches public key keynum");
	const key = createPublicKey({
		key: { kty: "OKP", crv: "Ed25519", x: pk.pk.toString("base64url") },
		format: "jwk"
	});
	const digest = createHash("blake2b512").update(fileBytes).digest();
	const signatureOk = edVerify(null, digest, key, sig.sig);
	const commentOk = edVerify(
		null,
		Buffer.concat([sig.sig, Buffer.from(sig.trustedComment, "utf8")]),
		key,
		sig.globalSig
	);
	return { signatureOk, commentOk, trustedComment: sig.trustedComment };
}

// ---------------------------------------------------------------------------
// offline verification-logic test: synthetic minisign artifacts built with
// node's own ed25519 (no tauri CLI needed)
// ---------------------------------------------------------------------------

test("verification logic accepts a synthetic minisign signature and rejects tampering", () => {
	const { publicKey, privateKey } = generateKeyPairSync("ed25519");
	const keynum = Buffer.from("01234567");
	const payload = Buffer.from("synthetic updater payload\n");
	const trustedComment = "timestamp:42\tsynthetic\tversion:9.9.9";

	const digest = createHash("blake2b512").update(payload).digest();
	const sig = edSign(null, digest, privateKey);
	const globalSig = edSign(
		null,
		Buffer.concat([sig, Buffer.from(trustedComment, "utf8")]),
		privateKey
	);

	const pubText = b64(
		`untrusted comment: minisign public key: TEST\n${b64(Buffer.concat([Buffer.from("Ed", "latin1"), keynum, publicKey.export({ type: "spki", format: "der" }).subarray(-32)]))}\n`
	);
	const sigText = b64(
		[
			"untrusted comment: signature from tauri secret key",
			b64(Buffer.concat([Buffer.from("ED", "latin1"), keynum, sig])),
			`trusted comment: ${trustedComment}`,
			b64(globalSig)
		].join("\n") + "\n"
	);

	const good = verifyUpdaterSignature(sigText, pubText, payload);
	assert.equal(good.signatureOk, true, "main signature verifies");
	assert.equal(good.commentOk, true, "trusted-comment signature verifies");
	assert.equal(good.trustedComment, trustedComment);

	const tampered = Buffer.from(payload);
	tampered[0] ^= 1;
	assert.equal(
		verifyUpdaterSignature(sigText, pubText, tampered).signatureOk,
		false,
		"modified file is rejected"
	);

	const lines = Buffer.from(sigText, "base64").toString("utf8").split("\n");
	const corruptedSig = Buffer.from(lines[1], "base64");
	corruptedSig[corruptedSig.length - 1] ^= 1;
	lines[1] = corruptedSig.toString("base64");
	const badSigText = b64(lines.join("\n"));
	assert.equal(
		verifyUpdaterSignature(badSigText, pubText, payload).signatureOk,
		false,
		"flipped signature bytes are rejected"
	);
});

// ---------------------------------------------------------------------------
// real-CLI fixture tests: throwaway keys in a temp dir, never the repo
// ---------------------------------------------------------------------------

function runCli(args, options = {}) {
	return spawnSync(tauriCli, args, { encoding: "utf8", ...options });
}

test(
	"tauri signer signs a fixture via the env the workflow uses, and the signature verifies",
	{ skip: !existsSync(tauriCli) ? "node_modules/.bin/tauri not installed" : false },
	(t) => {
		const dir = mkdtempSync(join(tmpdir(), "release-signing-test-"));
		t.after(() => rmSync(dir, { recursive: true, force: true }));
		const keyPath = join(dir, "throwaway.key");
		const fixturePath = join(dir, "Brainstory_0.4.1_aarch64.app.tar.gz");
		const payload = "throwaway updater fixture bytes\n";
		writeFileSync(fixturePath, payload);

		const generated = runCli(["signer", "generate", "-w", keyPath, "--force", "--ci"]);
		assert.equal(generated.status, 0, generated.stderr);

		// exactly the environment the release job's sign step provides
		const signed = runCli(["signer", "sign", "--app-version", "0.4.1", fixturePath], {
			env: {
				...process.env,
				TAURI_SIGNING_PRIVATE_KEY: readFileSync(keyPath, "utf8"),
				TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ""
			}
		});
		assert.equal(signed.status, 0, signed.stderr);

		const sigPath = `${fixturePath}.sig`;
		assert.ok(existsSync(sigPath), "the .sig sidecar is named exactly <artifact>.sig");

		const pubText = readFileSync(`${keyPath}.pub`, "utf8");
		const sigText = readFileSync(sigPath, "utf8").trim();
		assert.doesNotMatch(
			sigText,
			/\n/,
			"the .sig file is the single-line base64 the updater feed embeds"
		);

		const result = verifyUpdaterSignature(sigText, pubText, Buffer.from(payload));
		assert.equal(
			result.signatureOk,
			true,
			"main signature verifies with the generated public key"
		);
		assert.equal(result.commentOk, true, "trusted comment verifies");
		assert.match(result.trustedComment, /\tfile:Brainstory_0\.4\.1_aarch64\.app\.tar\.gz\t/);
		assert.match(
			result.trustedComment,
			/\tversion:0\.4\.1$/,
			"--app-version is bound into the signed comment"
		);

		const tampered = Buffer.from(payload);
		tampered[0] ^= 1;
		assert.equal(
			verifyUpdaterSignature(sigText, pubText, tampered).signatureOk,
			false,
			"modified artifact is rejected"
		);
	}
);

test(
	"a password-protected throwaway key signs through the password env; a wrong password fails",
	{ skip: !existsSync(tauriCli) ? "node_modules/.bin/tauri not installed" : false },
	(t) => {
		const dir = mkdtempSync(join(tmpdir(), "release-signing-test-"));
		t.after(() => rmSync(dir, { recursive: true, force: true }));
		const keyPath = join(dir, "throwaway.key");
		const fixturePath = join(dir, "Brainstory_0.4.1_x64-setup.exe");
		const payload = "password-protected key fixture\n";
		writeFileSync(fixturePath, payload);

		const generated = runCli([
			"signer",
			"generate",
			"-w",
			keyPath,
			"--force",
			"-p",
			"throwaway-passphrase"
		]);
		assert.equal(generated.status, 0, generated.stderr);

		const signed = runCli(["signer", "sign", fixturePath], {
			env: {
				...process.env,
				TAURI_SIGNING_PRIVATE_KEY: readFileSync(keyPath, "utf8"),
				TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "throwaway-passphrase"
			}
		});
		assert.equal(signed.status, 0, signed.stderr);

		const result = verifyUpdaterSignature(
			readFileSync(`${fixturePath}.sig`, "utf8").trim(),
			readFileSync(`${keyPath}.pub`, "utf8"),
			Buffer.from(payload)
		);
		assert.equal(
			result.signatureOk,
			true,
			"password-protected key produces a verifying signature"
		);

		const wrong = runCli(["signer", "sign", fixturePath], {
			env: {
				...process.env,
				TAURI_SIGNING_PRIVATE_KEY: readFileSync(keyPath, "utf8"),
				TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "wrong-passphrase"
			}
		});
		assert.notEqual(wrong.status, 0, "a wrong password must fail the sign step");
	}
);
