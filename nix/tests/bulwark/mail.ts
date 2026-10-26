import { readFileSync, rmSync, writeFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

function env(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is unset`);
  return value;
}

type Role = 'reader' | 'sender';
const read = (path: string) => readFileSync(path, 'utf8').trim();
const accounts: Record<Role, { address: string; passwordFile: string }> = JSON.parse(read(env('ACCOUNTS')));
const password = (role: Role) => read(accounts[role].passwordFile);

export const blobStore = {
  down: () => writeFileSync(env('S3_UNAVAILABLE'), ''),
  up: () => rmSync(env('S3_UNAVAILABLE'), { force: true }),
};

type Call = [string, Record<string, unknown>];

class Jmap {
  private constructor(
    private readonly auth: string,
    private readonly apiUrl: string,
    private readonly accountId: string,
  ) {}

  static async as(role: Role): Promise<Jmap> {
    const auth = `Basic ${Buffer.from(`${accounts[role].address}:${password(role)}`).toString('base64')}`;
    const response = await fetch(`${env('JMAP_URL')}/jmap/session`, { headers: { Authorization: auth } });
    expect(response.status, 'JMAP session').toBe(200);
    const session = await response.json();
    return new Jmap(auth, session.apiUrl, session.primaryAccounts['urn:ietf:params:jmap:mail']);
  }

  /** Makes `calls` in one request, in this account; call i's id is `${i}`. */
  async call(...calls: Call[]): Promise<any[]> {
    const response = await fetch(this.apiUrl, {
      method: 'POST',
      headers: { Authorization: this.auth, 'Content-Type': 'application/json' },
      body: JSON.stringify({
        using: ['urn:ietf:params:jmap:core', 'urn:ietf:params:jmap:mail', 'urn:ietf:params:jmap:submission'],
        methodCalls: calls.map(([name, args], i) => [name, { accountId: this.accountId, ...args }, `${i}`]),
      }),
    });
    expect(response.status, 'JMAP request').toBe(200);
    const { methodResponses } = await response.json();
    return methodResponses.map(([name, result]: [string, any], i: number) => {
      expect(name, JSON.stringify(result)).toBe(calls[i][0]);
      return result;
    });
  }
}

/** Mail from the sender to the reader, each message replying to the one before. */
export class Thread {
  readonly subject = `Thread ${Date.now()}`;
  private readonly messageIds: string[] = [];

  static async of(messages: number): Promise<Thread> {
    const thread = new Thread();
    for (let n = 0; n < messages; n++) await thread.add();
    await expect.poll(() => thread.delivered(), { timeout: 30_000 }).toHaveLength(messages);
    return thread;
  }

  body = (n: number) => `message ${n} of ${this.subject}`;

  /** Sends the next message, a second after the last, since receivedAt counts whole seconds. */
  async add(): Promise<void> {
    await new Promise((resolve) => setTimeout(resolve, 1100));
    const sender = await Jmap.as('sender');
    const [{ list: [identity] }, { ids: [sent] }] = await sender.call(
      ['Identity/get', {}],
      ['Mailbox/query', { filter: { role: 'sent' } }],
    );
    const n = this.messageIds.length + 1;
    const messageId = `${n}.${Date.now()}@${accounts.sender.address.split('@')[1]}`;
    const [email, submission] = await sender.call(
      ['Email/set', { create: { m: {
        mailboxIds: { [sent]: true },
        keywords: { $seen: true },
        from: [{ email: accounts.sender.address }],
        to: [{ email: accounts.reader.address }],
        subject: n === 1 ? this.subject : `Re: ${this.subject}`,
        messageId: [messageId],
        inReplyTo: n > 1 ? this.messageIds.slice(-1) : null,
        references: n > 1 ? this.messageIds : null,
        bodyValues: { body: { value: this.body(n) } },
        textBody: [{ partId: 'body', type: 'text/plain' }],
      } } }],
      ['EmailSubmission/set', { create: { s: { identityId: identity.id, emailId: '#m' } } }],
    );
    expect(email.created?.m, JSON.stringify(email)).toBeTruthy();
    expect(submission.created?.s, JSON.stringify(submission)).toBeTruthy();
    this.messageIds.push(messageId);
  }

  async delivered(): Promise<{ id: string; messageId: string[] }[]> {
    const [, { list }] = await (await Jmap.as('reader')).call(
      ['Email/query', { filter: { subject: this.subject }, sort: [{ property: 'receivedAt' }] }],
      ['Email/get', { '#ids': { resultOf: '0', name: 'Email/query', path: '/ids' }, properties: ['messageId'] }],
    );
    return list;
  }

  /** Marks the reader's copy of message n read, so the conversation opens with it folded. */
  async markRead(n: number): Promise<void> {
    const copy = (await this.delivered()).find((email) => email.messageId.includes(this.messageIds[n - 1]));
    const [result] = await (await Jmap.as('reader')).call(
      ['Email/set', { update: { [copy!.id]: { 'keywords/$seen': true } } }],
    );
    expect(result.updated, JSON.stringify(result)).toHaveProperty(copy!.id);
  }
}

export async function signIn(page: Page): Promise<void> {
  await page.goto('/');
  await page.locator('#username').fill(accounts.reader.address);
  await page.locator('#password').fill(password('reader'));
  await page.locator('button[type="submit"]').click();
  await expect(page.locator('[data-testid="account-switcher"]').first()).toBeVisible();
}

// a thread's row has no test hook of its own, so this is its subject line, the latest message's
export const row = (page: Page, thread: Thread) =>
  page.locator('[data-tour="email-list"]').getByText(`Re: ${thread.subject}`, { exact: true });
export const viewer = (page: Page) => page.locator('[data-tour="email-viewer"]');
// the conversation's other messages, folded to their sender and snippet or unfolded with their body
export const folded = (page: Page) => viewer(page).locator('[data-mv-conversation-card]');
export const unfolded = (page: Page) => viewer(page).locator('[data-mv-conversation-expanded]');

export const shot = (page: Page, name: string) =>
  page.screenshot({ path: test.info().outputPath(`${name}.png`), fullPage: true });
