import { mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";

// The smoke test drives the real debug binary through tauri-driver, the
// WebDriver wrapper around the platform webview (WebKitGTK on Linux).
// No model download, no LLM call: text in, message on screen.

const debugBinary = resolve(import.meta.dirname, "../src-tauri/target/debug/brainstory");
// A fresh data dir per run: the test must never touch a real database
// (a first-run app also skips any stale state the dev machine has).
const dataDir = mkdtempSync(join(tmpdir(), "brainstory-e2e-"));

const require = createRequire(import.meta.url);

let tauriDriver;

export const config = {
	runner: "local",
	specs: [resolve(import.meta.dirname, "smoke.spec.mjs")],
	maxInstances: 1,
	// tauri-driver speaks the WebDriver protocol on localhost:4444
	hostname: "127.0.0.1",
	port: 4444,
	path: "/",
	capabilities: [
		{
			maxInstances: 1,
			"tauri:options": { application: debugBinary }
		}
	],
	logLevel: "warn",
	bail: 1,
	framework: "mocha",
	mochaOpts: {
		ui: "bdd",
		// the debug binary under xvfb/llvmpipe paints slowly; islands
		// hydrate seconds later
		timeout: 240_000
	},
	reporters: ["spec"],
	afterTest(test, context, { error }) {
		// one screenshot per failure, next to the spec (uploaded by CI);
		// the dir is created here so a fresh checkout works
		if (!error) return;
		const dir = resolve(import.meta.dirname, "screenshots");
		mkdirSync(dir, { recursive: true });
		return browser
			.getUrl()
			.then((url) => console.error(`failing page: ${url}`))
			.catch(() => {})
			.then(() =>
				browser.saveScreenshot(resolve(dir, `${Date.now()}-failure.png`))
			);
	},
	//
	// Start tauri-driver before the session; the app it spawns inherits
	// this environment, so XDG_DATA_HOME redirects the database.
	onPrepare() {
		process.env.XDG_DATA_HOME = join(dataDir, "data");
		process.env.XDG_CACHE_HOME = join(dataDir, "cache");
		tauriDriver = spawn("tauri-driver", [], { stdio: "inherit" });
	},
	onComplete() {
		tauriDriver?.kill("SIGTERM");
		// keep the temp dir on failure for post-mortem; clean it otherwise
		if (process.env.E2E_KEEP_DATA !== "1") {
			rmSync(dataDir, { recursive: true, force: true });
		}
	}
};
