import { existsSync } from 'node:fs';
import { isAbsolute } from 'node:path';
import { expect, test, type Response } from '@playwright/test';
import { env, shot, signIn } from './mail';

const isAdminLogin = (response: Response) =>
  response.request().method() === 'POST' && new URL(response.url()).pathname === '/api/admin/auth';

test('a Stalwart admin opens the admin dashboard without its password', async ({ page }) => {
  const statuses: number[] = [];
  page.on('response', (response) => isAdminLogin(response) && statuses.push(response.status()));
  const login = page.waitForResponse(isAdminLogin);
  await signIn(page, 'admin');
  await page.locator('a[title="Admin"]').click();

  const response = await login;
  expect(response.status(), await response.text()).toBe(200);
  await expect(page.getByText('Recent Activity')).toBeVisible();
  await shot(page, 'dashboard');
  expect(new Set(statuses)).toEqual(new Set([200]));

  const stateDir = env('ADMIN_STATE_DIR');
  expect(isAbsolute(stateDir) && existsSync(stateDir), stateDir).toBe(true);
});
