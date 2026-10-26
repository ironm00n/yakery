import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  outputDir: process.env.OUT,
  timeout: 180_000,
  expect: { timeout: 30_000 },
  workers: 1,
  reporter: 'list',
  use: {
    baseURL: process.env.WEBMAIL_URL,
    viewport: { width: 1440, height: 900 },
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
});
