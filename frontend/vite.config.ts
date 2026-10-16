import path from 'node:path';
import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// The Rust backend (crates/parcel-server) listens on :8106 in local dev.
// Override to point the dev proxy elsewhere, e.g. PARCEL_BACKEND_URL=http://localhost:9000.
const BACKEND = process.env.PARCEL_BACKEND_URL ?? 'http://localhost:8106';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    port: 5178,
    strictPort: true,
    // prox serves the app at https://parcel.local.stridelabs.ai -> Vite :5178.
    allowedHosts: ['.local.stridelabs.ai'],
    proxy: {
      '/api': { target: BACKEND, changeOrigin: true },
      // Server-side sign-in (login, callback, logout): redirects and cookies.
      '/auth/': { target: BACKEND, changeOrigin: true },
      '/openapi.json': { target: BACKEND, changeOrigin: true },
      '/docs': { target: BACKEND, changeOrigin: true },
    },
  },
  build: {
    // Served by the Rust backend (PARCEL_STATIC_DIR) with an index.html SPA
    // fallback, so assets keep Vite's default absolute `/assets/...` paths.
    outDir: 'dist',
    rollupOptions: {
      output: {
        // Split stable vendor code from the app so deploys don't bust it.
        manualChunks(id) {
          if (!id.includes('node_modules')) return undefined;
          if (
            /[\\/]node_modules[\\/](react|react-dom|scheduler|react-router|react-router-dom)[\\/]/.test(
              id,
            )
          ) {
            return 'vendor-react';
          }
          if (id.includes('@radix-ui') || id.includes('@floating-ui')) return 'vendor-radix';
          if (id.includes('@tanstack')) return 'vendor-query';
          return 'vendor';
        },
      },
    },
  },
});
