/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { tanstackRouter } from '@tanstack/router-plugin/vite'
import { playwright } from '@vitest/browser-playwright'
import { fileURLToPath } from 'node:url'

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    tanstackRouter({
      target: 'react',
      autoCodeSplitting: true,
    }),
    react(),
    tailwindcss(),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  optimizeDeps: {
    // Pre-bundle the UI/test runtime up front. Without this, the first test
    // run after a fresh install discovers dependencies mid-run and reloads
    // Vite, which can leave a test with a second React copy.
    include: [
      'zustand',
      'axios',
      'zod',
      'react-hook-form',
      '@hookform/resolvers/zod',
      '@tanstack/react-query',
      '@tanstack/react-router',
      '@tanstack/react-table',
      '@radix-ui/react-avatar',
      '@radix-ui/react-checkbox',
      '@radix-ui/react-dialog',
      '@radix-ui/react-dropdown-menu',
      '@radix-ui/react-label',
      '@radix-ui/react-popover',
      '@radix-ui/react-radio-group',
      '@radix-ui/react-scroll-area',
      '@radix-ui/react-select',
      '@radix-ui/react-separator',
      '@radix-ui/react-tooltip',
      'cmdk',
      'lucide-react',
      'sonner',
      'vitest-browser-react',
    ],
  },
  server: {
    // Keep local development same-origin: the API client uses relative paths
    // and the backend listens on 8080.
    proxy: {
      '/api/v1': 'http://127.0.0.1:8080',
      '/health': 'http://127.0.0.1:8080',
    },
  },
  test: {
    silent: 'passed-only',
    unstubEnvs: true,
    setupFiles: ['./src/test-utils/setup.ts'],
    browser: {
      enabled: true,
      provider: playwright(),
      instances: [{ browser: 'chromium' }],
    },
    coverage: {
      // include: ['src/**/*.{js,jsx,ts,tsx}'], // Uncomment to expand the report to all src/**/* so untested modules appear as 0% coverage.
      exclude: [
        'src/api/generated/**',
        'src/components/ui/**',
        'src/assets/**',
        'src/tanstack-table.d.ts',
        'src/routeTree.gen.ts',
        'src/test-utils/**',
        'src/routes/**',
      ],
    },
  },
})
