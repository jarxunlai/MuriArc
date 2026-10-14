import { defineConfig, devices } from '@playwright/test'

const isCi = Boolean(process.env.CI)

// Runs against an already started Server (Compose stack) with the production
// remote gateway. Unlike playwright.config.ts it never starts Vite or DemoGateway,
// so these scenarios are end-to-end evidence for the real REST/Store path.
export default defineConfig({
  testDir: './e2e-server',
  timeout: 60_000,
  expect: { timeout: 15_000 },
  fullyParallel: false,
  workers: 1,
  forbidOnly: isCi,
  retries: isCi ? 1 : 0,
  reporter: isCi
    ? [['line'], ['html', { open: 'never', outputFolder: 'playwright-report/server' }]]
    : [['list']],
  outputDir: 'test-results/server',
  use: {
    baseURL: process.env.MURIARC_E2E_BASE_URL ?? 'http://127.0.0.1:8787',
    reducedMotion: 'reduce',
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  projects: [
    {
      name: 'server-chromium',
      use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 960 } },
    },
  ],
})
