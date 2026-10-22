import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Puerto fijo: Tauri carga la interfaz desde aquí en modo desarrollo.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
});
