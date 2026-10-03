import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import process from "node:process";
import { describe, expect, it } from "vitest";

// vitest runs from the frontend package root (vitest.config.mjs)
const root = process.cwd();
const read = (path: string): string => readFileSync(resolve(root, path), "utf8");

describe("global.css font stack", () => {
	it("names the family that @fontsource-variable/inter actually registers first", () => {
		const fontCss = read("node_modules/@fontsource-variable/inter/index.css");
		const registered = /font-family:\s*['"]([^'"]+)['"]/.exec(fontCss)?.[1];
		expect(registered).toBe("Inter Variable");

		// Tailwind's preflight puts --font-sans on <html>; if its first
		// family is not the loaded one, the whole app renders in system-ui
		const css = read("src/styles/global.css");
		const fontSans = /--font-sans:\s*([^;]+);/.exec(css)?.[1] ?? "";
		const firstFamily = /^\s*['"]([^'"]+)['"]/.exec(fontSans)?.[1];
		expect(firstFamily).toBe(registered);
	});
});
