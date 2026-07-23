import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    target: "es2022",
    sourcemap: true,
  },
  test: {
    coverage: {
      provider: "v8",
      reporter: ["text", "html", "lcov"],
      include: ["src/**/*.{ts,svelte}"],
      exclude: ["src/**/*.test.ts", "src/vite-env.d.ts"],
      thresholds: {
        lines: 10,
        statements: 10,
        functions: 10,
        branches: 14,
      },
    },
  },
});
