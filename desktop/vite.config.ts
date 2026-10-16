import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
export default defineConfig({
  plugins: [svelte()],
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          // Language packages depend on the editor runtime; keep that boundary acyclic.
          if (
            /\/node_modules\/(?:@codemirror\/lang-|@lezer\/(?:markdown|html|css|javascript)\/)/.test(
              id,
            )
          )
            return "editor-languages";
          if (/\/node_modules\/(?:@codemirror|@lezer)\//.test(id))
            return "editor";
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
    setupFiles: ["./src/test-setup.ts"],
  },
  resolve: { conditions: ["browser"] },
});
