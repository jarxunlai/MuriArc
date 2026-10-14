import { fileURLToPath, URL } from 'node:url'
import vue from '@vitejs/plugin-vue'
import { loadEnv } from 'vite'
import { defineConfig } from 'vitest/config'

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), 'VITE_')
  // Replaced before bundling so production builds tree-shake the demo transport.
  const demoGatewayAvailable = mode !== 'production' || env.VITE_MURIARC_GATEWAY === 'demo'

  return {
    plugins: [vue()],
    define: {
      __MURIARC_DEMO_GATEWAY__: JSON.stringify(demoGatewayAvailable),
    },
    resolve: {
      alias: {
        '@': fileURLToPath(new URL('./src', import.meta.url)),
      },
    },
    server: {
      port: 1420,
      strictPort: true,
    },
    preview: {
      port: 4173,
    },
    build: {
      target: ['es2021', 'chrome100', 'safari13'],
      sourcemap: true,
    },
    test: {
      include: ['./src/**/*.test.ts'],
      environment: 'jsdom',
      globals: true,
      setupFiles: ['./src/test-setup.ts'],
    },
  }
})
