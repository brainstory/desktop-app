import js from "@eslint/js";
import globals from "globals";
import eslintReact from "@eslint-react/eslint-plugin";
import reactHooks from "eslint-plugin-react-hooks";
import jsxA11y from "eslint-plugin-jsx-a11y-x";
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
			// the maintained jsx-a11y fork (the original stopped at ESLint 9);
			// eslint-plugin-astro wraps the same rules as astro/jsx-a11y/*
			jsxA11y.configs.recommended,
			// eslint-plugin-react stopped at ESLint 9 (it calls the removed
			// context.getFilename); @eslint-react is the maintained successor
			eslintReact.configs["recommended-type-checked"]
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
			"react-hooks": reactHooks
		},
		rules: {
			...reactHooks.configs.recommended.rules,
			// eslint-plugin-react-hooks (the React team's) stays the source of
			// truth for hooks; turn off @eslint-react's re-implementations so
			// each problem is reported once
			"@eslint-react/error-boundaries": "off",
			"@eslint-react/exhaustive-deps": "off",
			"@eslint-react/purity": "off",
			"@eslint-react/rules-of-hooks": "off",
			"@eslint-react/set-state-in-effect": "off",
			"@eslint-react/set-state-in-render": "off",
			"@eslint-react/static-components": "off",
			"@eslint-react/unsupported-syntax": "off",
			"@eslint-react/use-memo": "off",
			// ion-icon is a web component; its `class` attribute is fine
			"@eslint-react/dom-no-unknown-property": ["error", { ignore: ["class"] }],
			// Style rules @eslint-react adds that eslint-plugin-react never
			// enforced. Off to keep the ESLint 10 move a like-for-like swap;
			// adopting them is a separate cleanup:
			// - React 19 idioms (use() over useContext, <Ctx> over
			//   <Ctx.Provider>, ref-as-prop over forwardRef)
			"@eslint-react/no-use-context": "off",
			"@eslint-react/no-context-provider": "off",
			"@eslint-react/no-forward-ref": "off",
			// - naming conventions for useId/useRef results
			"@eslint-react/naming-convention-id-name": "off",
			"@eslint-react/naming-convention-ref-name": "off",
			// - lazy useState initialisers (the flagged ones are cheap reads)
			"@eslint-react/use-state": "off",
			// - index keys: the flagged lists are static or re-rendered whole
			"@eslint-react/no-array-index-key": "off",
			// - the tabs and tooltip primitives use the Children API and
			//   cloneElement on purpose (to wire ids and aria-* onto children)
			"@eslint-react/no-children-count": "off",
			"@eslint-react/no-children-for-each": "off",
			"@eslint-react/no-children-map": "off",
			"@eslint-react/no-children-to-array": "off",
			"@eslint-react/no-clone-element": "off",
			// keep the console clean in production
			"no-console": ["warn", { allow: ["error", "warn"] }],
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
