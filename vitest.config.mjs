import { defineConfig } from "vitest/config";
import tsconfigPaths from "vite-tsconfig-paths";

// path aliases come from tsconfig.json (vite-tsconfig-paths): the
// single source of truth the app code itself is checked against
export default defineConfig({
	plugins: [tsconfigPaths()],
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
				// floors just under the current suite (88/86/83/79):
				// a drop below one is a real regression worth failing on
				lines: 85,
				statements: 84,
				functions: 80,
				branches: 75
			}
		}
	}
});
