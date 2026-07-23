import { defineConfig } from "eslint/config";
import svelte from "eslint-plugin-svelte";
import ts from "typescript-eslint";

const caps = {
  complexity: ["error", 15],
  "max-lines-per-function": [
    "error",
    { max: 100, skipBlankLines: true, skipComments: true },
  ],
  "max-depth": ["error", 4],
};

export default defineConfig(
  { ignores: ["coverage/", "dist/", "target/"] },
  {
    files: ["**/*.ts"],
    languageOptions: { parser: ts.parser },
    rules: caps,
  },
  ...svelte.configs.base,
  {
    files: ["**/*.svelte"],
    languageOptions: { parserOptions: { parser: ts.parser } },
    rules: caps,
  },
  {
    files: ["**/*.test.ts", "tests/**"],
    rules: { "max-lines-per-function": "off" },
  },
);
