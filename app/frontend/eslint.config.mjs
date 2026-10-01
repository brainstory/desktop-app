import js from "@eslint/js";
import globals from "globals";
import react from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";
import jsxA11y from "eslint-plugin-jsx-a11y";
import astro from "eslint-plugin-astro";
import tseslint from "typescript-eslint";

export default tseslint.config(
	{
		ignores: ["dist/**", "node_modules/**", ".astro/**", "public/vendor/**"]
	},
	js.configs.recommended,
	// type-aware rules (no-floating-promises, no-misused-promises, ...) for
	// the app's TypeScript; .astro files and node scripts get the untyped
	// set below
	{
		files: ["**/*.{ts,tsx}"],
		extends: [
			tseslint.configs.recommendedTypeChecked,
			// same plugin object eslint-plugin-astro registers for its
			// a11y rules (flatConfigs carries a copy, and flat config
			// refuses two different "jsx-a11y" plugins)
			{ ...jsxA11y.flatConfigs.recommended, plugins: { "jsx-a11y": jsxA11y } }
		],
		languageOptions: {
			globals: { ...globals.browser, ...globals.node },
			parserOptions: {
				ecmaVersion: "latest",
				sourceType: "module",
				ecmaFeatures: { jsx: true },
				projectService: true,
				tsconfigRootDir: import.meta.dirname
			}
		},
		plugins: {
			react,
			"react-hooks": reactHooks
		},
		settings: { react: { version: "detect" } },
		rules: {
			...react.configs.flat.recommended.rules,
			...reactHooks.configs.recommended.rules,
			// React 19 JSX transform: no React import needed
			"react/react-in-jsx-scope": "off",
			"react/jsx-uses-react": "off",
			// ion-icon is a web component; unknown DOM props are fine
			"react/no-unknown-property": ["error", { ignore: ["class"] }],
			// keep the console clean in production
			"no-console": ["warn", { allow: ["error", "warn"] }],
			// prop-type declarations are superseded by TypeScript
			"react/prop-types": "off",
			// allow the conventional `_` name for intentionally unused vars
			"no-unused-vars": "off",
			"@typescript-eslint/no-unused-vars": [
				"error",
				{ argsIgnorePattern: "^_", varsIgnorePattern: "^_" }
			],
			// Tauri commands reject with plain strings: helpers that pass an
			// upstream rejection along must keep it as-is (unknown)
			"@typescript-eslint/prefer-promise-reject-errors": [
				"error",
				{ allowThrowingUnknown: true }
			]
		}
	},
	// tests model the real IPC: Tauri rejects with strings, and the mocks
	// are async functions standing in for async commands
	{
		files: ["**/*.test.{ts,tsx}", "src/test/**/*.{ts,tsx}"],
		rules: {
			"@typescript-eslint/only-throw-error": "off",
			"@typescript-eslint/require-await": "off"
		}
	},
	// .astro pages and layouts: Astro's recommended rules (incl. a11y)
	...astro.configs["flat/recommended"],
	...astro.configs["flat/jsx-a11y-recommended"],
	{
		files: ["**/*.astro"],
		languageOptions: {
			globals: { ...globals.browser },
			parserOptions: { parser: tseslint.parser, extraFileExtensions: [".astro"] }
		}
	},
	// plain node scripts: untyped rules only (no project service)
	{
		files: ["scripts/**/*.mjs"],
		extends: [tseslint.configs.disableTypeChecked],
		languageOptions: {
			globals: { ...globals.node },
			parserOptions: { ecmaVersion: "latest", sourceType: "module" }
		}
	}
);
