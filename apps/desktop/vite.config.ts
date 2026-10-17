import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  test: {
    include: ['tests/**/*.test.ts', 'tests/**/*.test.tsx'],
    environment: 'jsdom',
    setupFiles: ['tests/setup.ts'],
  },
  server: { host: '127.0.0.1', port: 1420, strictPort: true },
  build: { target: 'safari18.5', sourcemap: false },
});
