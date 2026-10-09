/// <reference types="vite/client" />

/**
 * Text imports. The manual window reads `docs/MANUAL.md` this way so the
 * file the app shows and the file the repository carries are the same one.
 */
declare module '*.md?raw' {
  const content: string
  export default content
}

// Tauri's own core module, under the name src/lib/timedCore.ts imports it
// by (vite.config.ts): '@tauri-apps/api/core' itself points at timedCore.
declare module 'tauri-api-core-original' {
  export * from '@tauri-apps/api/core'
}
