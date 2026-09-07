import { defineConfig } from "vite";
export default defineConfig(({ mode }) => ({
  base: "./",
  build: {
    outDir: mode === "preview" ? "dist-preview" : "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: [mode === "preview" ? "preview.html" : "index.html", "print.html"],
    },
  },
  server: { host: "127.0.0.1", port: 5180, strictPort: true },
}));
