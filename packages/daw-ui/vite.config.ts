import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react-swc'

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: 'esnext',
    minify: !process.env.TAURI_DEBUG ? 'esbuild' : false,
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
