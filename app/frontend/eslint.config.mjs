import js from "@eslint/js";
import globals from "globals";
import react from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";

export default [
	{
		ignores: [
			"dist/**",
			"node_modules/**",
			".astro/**",
			"public/vendor/**",
			"src/**/*.test.js"
		]
	},
	js.configs.recommended,
	{
		files: ["**/*.{js,mjs,jsx}"],
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
			// the codebase is untyped JS; prop-type declarations would be
			// pure ceremony until a TypeScript migration happens
			"react/prop-types": "off",
			// allow the conventional `_` name for intentionally unused args
			"no-unused-vars": [
				"error",
				{ argsIgnorePattern: "^_", varsIgnorePattern: "^_" }
			]
		}
	}
];
