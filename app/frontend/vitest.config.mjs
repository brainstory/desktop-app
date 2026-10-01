import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

// Mirrors the path aliases in tsconfig.json so tests can import through
// the same "@..." specifiers the app code uses.
const resolveFromRoot = (p) => fileURLToPath(new URL(p, import.meta.url));

export default defineConfig({
	resolve: {
		alias: {
			"@src": resolveFromRoot("./src"),
			"@components": resolveFromRoot("./src/components"),
			"@helpers": resolveFromRoot("./src/helpers"),
			"@layouts": resolveFromRoot("./src/layouts"),
			"@ds": resolveFromRoot("./src/components/global/design-system")
		}
	},
	test: {
		environment: "jsdom",
		include: ["src/**/*.test.{ts,tsx}"],
		setupFiles: ["src/test/setup.ts"],
		coverage: {
			provider: "v8",
			include: ["src/**/*.{ts,tsx}"],
			exclude: [
				"src/**/*.test.{ts,tsx}",
				// test harness, not app code
				"src/test/**",
				// declarations and type-only modules: nothing to execute
				"src/**/*.d.ts",
				"src/types.ts",
				"src/components/chat/types.ts"
			],
			thresholds: {
				// starting floor; raise as component coverage grows
				lines: 50
			}
		}
	}
});
