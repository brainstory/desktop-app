import js from "@eslint/js";
import globals from "globals";
import react from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
	{
		ignores: ["dist/**", "node_modules/**", ".astro/**", "public/vendor/**"]
	},
	js.configs.recommended,
	...tseslint.configs.recommended,
	{
		files: ["**/*.{js,mjs,jsx,ts,tsx}"],
		// build scripts are plain node, not type-checked app code
		ignores: ["scripts/**"],
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
			]
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
