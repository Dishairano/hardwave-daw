/// <reference types="vite/client" />

/**
 * Text imports. The manual window reads `docs/MANUAL.md` this way so the
 * file the app shows and the file the repository carries are the same one.
 */
declare module '*.md?raw' {
  const content: string
  export default content
}
