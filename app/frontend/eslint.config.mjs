import js from "@eslint/js";
import globals from "globals";
import react from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
	{
		ignores: [
			"dist/**",
			"node_modules/**",
			".astro/**",
			"public/vendor/**",
			"src/**/*.test.ts"
		]
	},
	js.configs.recommended,
	...tseslint.configs.recommended,
	{
		files: ["**/*.{js,mjs,jsx,ts,tsx}"],
		languageOptions: {
			globals: { ...globals.browser, ...globals.node },
			parserOptions: {
				ecmaVersion: "latest",
				sourceType: "module",
				ecmaFeatures: { jsx: true }
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
			"react/no-unknown-property": "off",
			// prop-type declarations are superseded by TypeScript
			"react/prop-types": "off",
			// allow the conventional `_` name for intentionally unused vars
			"no-unused-vars": "off",
			"@typescript-eslint/no-unused-vars": [
				"error",
				{ argsIgnorePattern: "^_", varsIgnorePattern: "^_" }
			]
		}
	}
);
