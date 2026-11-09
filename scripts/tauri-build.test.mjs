import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

function run(t, failure, succeedsOn = 99) {
	const dir = mkdtempSync(join(tmpdir(), "tauri-build-test-"));
	t.after(() => rmSync(dir, { recursive: true, force: true }));
	writeFileSync(
		join(dir, "pnpm"),
		`#!/usr/bin/env bash
n=0
[ ! -f "$STATE" ] || n=$(cat "$STATE")
n=$((n + 1))
echo "$n" > "$STATE"
printf '%s\\n' "$@" > "$ARGS"
[ "$n" -lt "$SUCCEEDS_ON" ] || exit 0
echo "$FAILURE" >&2
exit 42
`,
		{ mode: 0o755 }
	);
	writeFileSync(join(dir, "sleep"), "#!/usr/bin/env bash\nexit 0\n", { mode: 0o755 });
	const state = join(dir, "state");
	const args = join(dir, "args");
	const result = spawnSync(
		"bash",
		[resolve("scripts/tauri-build.sh"), "--bundles", "deb", "rpm", "appimage", "--no-sign"],
		{
			encoding: "utf8",
			env: {
				...process.env,
				PATH: `${dir}:${process.env.PATH}`,
				STATE: state,
				ARGS: args,
				FAILURE: failure,
				SUCCEEDS_ON: String(succeedsOn)
			}
		}
	);
	assert.ifError(result.error);
	assert.equal(
		readFileSync(args, "utf8"),
		"tauri\nbuild\n--bundles\ndeb\nrpm\nappimage\n--no-sign\n"
	);
	return { ...result, attempts: Number(readFileSync(state, "utf8")) };
}

test("build success needs one attempt and preserves arguments", (t) => {
	const result = run(t, "", 1);
	assert.equal(result.status, 0);
	assert.equal(result.attempts, 1);
});
test("transient bundler download can recover", (t) => {
	const result = run(t, "failed to bundle project: `http status: 504`", 2);
	assert.equal(result.status, 0);
	assert.equal(result.attempts, 2);
});
test("persistent gateway failure is bounded and retains exit status", (t) => {
	const result = run(t, "failed to bundle project: `http status: 502`");
	assert.equal(result.status, 42);
	assert.equal(result.attempts, 3);
});
for (const failure of [
	"error: could not compile transcribe-rs",
	"failed to bundle project: invalid config",
	"failed to bundle project: `http status: 404`"
]) {
	test(`non-transient failure is not retried: ${failure}`, (t) => {
		const result = run(t, failure);
		assert.equal(result.status, 42);
		assert.equal(result.attempts, 1);
	});
}
