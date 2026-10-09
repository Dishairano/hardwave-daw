import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

// Tauri's core module on disk, so the app's imports of it can go through
// src/lib/timedCore.ts (which times calls for the FPS meter) and that one
// file can still reach the real thing.
const tauriApiDir = dirname(createRequire(import.meta.url).resolve('@tauri-apps/api/package.json'))

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: [
      { find: /^@tauri-apps\/api\/core$/, replacement: fileURLToPath(new URL('./src/lib/timedCore.ts', import.meta.url)) },
      { find: /^tauri-api-core-original$/, replacement: join(tauriApiDir, 'core.js') },
    ],
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    fs: {
      // The manual lives at the repository root and is imported as text
      // by the in-app manual window, so one file is both what the app
      // shows and what the repository carries.
      allow: ['..', '../..'],
    },
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: 'esnext',
    minify: !process.env.TAURI_DEBUG,
    sourcemap: !!process.env.TAURI_DEBUG,
    // Push known-heavy / rarely-needed features into their own chunks so
    // the initial JS payload stays under ~400 KB gzipped. Vite emits each
    // matched module as a separate chunk that the runtime fetches lazily
    // when its consumer first imports it.
    rollupOptions: {
      output: {
        manualChunks(id) {
          // Plug-in picker — only loaded after user opens an FX slot
          if (id.includes('/plugin-picker/')) return 'plugin-picker'
          // KickSynth editor — heavy SVG + canvas, only opens on demand
          if (id.includes('KickSynthEditor')) return 'kicksynth-editor'
          // Beat slicer — separate route entirely
          if (id.includes('/beat-slicer/')) return 'beat-slicer'
          // Sample editor — separate route
          if (id.includes('/sample-editor/')) return 'sample-editor'
          // Piano roll — large, used per-clip not always present
          if (id.includes('/piano-roll/')) return 'piano-roll'
          // New mixer — only when experimental flag on. Keeps legacy
          // mixer load path unaffected by the new code.
          if (id.includes('/mixer/v2/')) return 'mixer-v2'
          // node_modules → vendor chunk so app code can iterate without
          // busting the vendor cache.
          if (id.includes('node_modules')) {
            if (id.includes('@tanstack/react-virtual')) return 'vendor-virtual'
            if (id.includes('react-dom')) return 'vendor-react-dom'
            if (id.includes('react/') || id.includes('react@')) return 'vendor-react'
            if (id.includes('zustand')) return 'vendor-zustand'
            if (id.includes('@tauri-apps')) return 'vendor-tauri'
            return 'vendor'
          }
        },
      },
    },
    chunkSizeWarningLimit: 600,
  },
})
