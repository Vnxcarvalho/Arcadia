import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    globals: true,
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
    setupFiles: ['./src/test/setup.ts'],
    // Node 26 ships its own `localStorage`, which stays undefined without a backing file and
    // hides the one jsdom provides. Turn it off so the tests get jsdom's.
    execArgv: ['--no-experimental-webstorage'],
  },
})
