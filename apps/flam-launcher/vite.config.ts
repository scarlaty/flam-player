import react from "@vitejs/plugin-react";
import { configDefaults, defineConfig } from "vitest/config";

// Port fixe : Tauri (devUrl) s'y connecte en mode développement.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    css: false,
    // Les tests de scripts/ utilisent node:test (npm run test:scripts).
    exclude: [...configDefaults.exclude, "scripts/**"],
  },
});
