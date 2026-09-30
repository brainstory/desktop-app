import { readFileSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Contract test: every Tauri command invoked from the frontend must be
 * registered in the Rust invoke_handler. A rename on either side
 * otherwise only fails at runtime.
 */

// file is app/frontend/src/helpers/api/contract.test.ts
const here = dirname(fileURLToPath(import.meta.url));
const srcRoot = resolve(here, "../.."); // app/frontend/src
const repoRoot = resolve(here, "../../../../..");

function walk(dir: string, files: string[] = []): string[] {
	for (const name of readdirSync(dir)) {
		const full = resolve(dir, name);
		if (statSync(full).isDirectory()) {
			walk(full, files);
		} else if (/\.(ts|tsx)$/.test(name)) {
			files.push(full);
		}
	}
	return files;
}

// the contract module resolves COMMANDS.camel -> snake_case names
const COMMANDS_TS = readFileSync(resolve(srcRoot, "tauri/commands.ts"), "utf8");
const cmdMap = new Map<string, string>();
for (const m of COMMANDS_TS.matchAll(/(\w+):\s*"([a-z_]+)"/g)) {
	cmdMap.set(m[1], m[2]);
}

// matches invoke("cmd") and invoke(COMMANDS.camel)
const INVOKE_RE = /invoke(?:<[^>]*>)?\(\s*(?:"([a-z_]+)"|COMMANDS\.(\w+))/g;

describe("frontend <-> rust IPC contract", () => {
	it("every invoked command is registered in generate_handler!", () => {
		const files = [
			...walk(resolve(srcRoot, "helpers/api")),
			...walk(resolve(srcRoot, "components"))
		].filter((f) => !f.endsWith("contract.test.ts"));

		const used = new Set<string>();
		for (const file of files) {
			const source = readFileSync(file, "utf8");
			for (const match of source.matchAll(INVOKE_RE)) {
				const resolved = match[1] ?? cmdMap.get(match[2] ?? "");
				if (resolved) {
					used.add(resolved);
				}
			}
		}
		expect(used.size).toBeGreaterThan(10);

		const lib = readFileSync(resolve(repoRoot, "src-tauri/src/lib.rs"), "utf8");
		const block = lib.match(/generate_handler!\[([^\]]*)\]/)?.[1] ?? "";
		const registered = new Set(
			[...block.matchAll(/([a-z_]+)\s*,/g)].map((m) => m[1].split("::").pop()!)
		);
		expect(registered.size).toBeGreaterThan(10);

		const missing = [...used].filter((command) => !registered.has(command));
		expect(
			missing,
			`frontend invokes commands the Rust side never registered: ${missing.join(", ")}`
		).toEqual([]);
	});
});
