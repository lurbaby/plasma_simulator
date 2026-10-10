import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // wasm-pack output will be linked as a local dependency
  optimizeDeps: {
    exclude: ['tok-sym-core'],
  },
  // The hackathon API (datasets, test runs, live model) lives in the
  // `fusionsim serve` process; the dev server forwards to it.
  server: {
    proxy: { '/api': 'http://127.0.0.1:9999' },
  },
  build: {
    chunkSizeWarningLimit: 1500, // WASM bundle is large; suppress warning
  },
  define: {
    __APP_VERSION__: JSON.stringify(process.env.npm_package_version),
  },
})
