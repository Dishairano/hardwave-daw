import { defineConfig } from 'vitest/config'

// Unit tests only — Playwright owns tests/*.spec.ts and the in-app dev
// harness owns src/dev; both would otherwise be swept up by vitest's
// default include glob.
export default defineConfig({
  test: {
    include: ['src/**/*.test.{ts,tsx}'],
    environment: 'node',
  },
})
