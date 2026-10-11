import { expect, test } from '@playwright/test';
import { basicAuth, env } from './mail';

test('the mobile app finds JMAP at the webmail address, on that host, sandboxed', async () => {
  const webmail = new URL(env('WEBMAIL_URL'));
  const auth = { Authorization: basicAuth('reader') };

  const discovery = await fetch(new URL('/.well-known/jmap', webmail), { headers: auth, redirect: 'manual' });
  expect(discovery.status, 'discovery redirects').toBeGreaterThanOrEqual(300);
  expect(discovery.status, 'discovery redirects').toBeLessThan(400);
  const session = new URL(discovery.headers.get('location') ?? '', webmail);
  expect(session.host, 'the redirect stays on the webmail host').toBe(webmail.host);

  const response = await fetch(session, { headers: auth });
  expect(response.status, 'JMAP session').toBe(200);
  expect((await response.json()).primaryAccounts['urn:ietf:params:jmap:mail']).toBeTruthy();
  expect(response.headers.get('content-security-policy')).toBe('sandbox');
});
