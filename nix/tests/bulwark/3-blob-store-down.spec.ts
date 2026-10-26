import { writeFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { Thread, blobStore, row, shot, signIn, viewer } from './mail';

const readerStates = (page: Page) => ({
  skeleton: page.locator('[class~="bg-muted/60"][class~="rounded-full"]'),
  'no conversation': page.getByRole('heading', { name: 'No conversation selected' }),
  conversation: viewer(page),
});

async function readerShows(page: Page): Promise<string> {
  for (const [state, locator] of Object.entries(readerStates(page))) {
    if (await locator.first().isVisible()) return state;
  }
  return 'none of these';
}

/** Each state the reader passes through after `act`, until one other than the skeleton holds for 5 s. */
async function watchReader(page: Page, act: () => Promise<void>) {
  const timeline = [{ state: await readerShows(page), ms: 0 }];
  const start = Date.now();
  await act();
  for (let now = 0; now < 90_000; now = Date.now() - start) {
    const last = timeline[timeline.length - 1];
    if (timeline.length > 1 && last.state !== 'skeleton' && now - last.ms > 5_000) break;
    const state = await readerShows(page);
    if (state !== last.state) {
      timeline.push({ state, ms: now });
      await shot(page, `${timeline.length}-${state.replace(/ /g, '-')}`);
    }
    await page.waitForTimeout(50);
  }
  return timeline;
}

// Known bad: the reader as cx33-1's showed it on 2026-10-26, kept until what it should do instead is decided.
test('with the blob store down, opening a thread shows a skeleton and then no conversation', async ({ page }) => {
  const thread = await Thread.of(2);
  await signIn(page);
  await expect(row(page, thread)).toBeVisible();
  const consoleErrors: string[] = [];
  const jmapErrors: unknown[] = [];
  page.on('console', (message) => message.type() === 'error' && consoleErrors.push(message.text()));
  page.on('response', async (response) => {
    if (!response.url().endsWith('/jmap/')) return;
    const { methodResponses = [] } = await response.json().catch(() => ({}));
    for (const [name, result] of methodResponses) if (name === 'error') jmapErrors.push(result);
  });

  blobStore.down();
  const timeline = await watchReader(page, () => row(page, thread).click()).finally(blobStore.up);
  const text = await readerStates(page)['no conversation'].locator('..').innerText().catch(() => null);
  const record = JSON.stringify({ timeline, text, consoleErrors, jmapErrors }, null, 2);
  writeFileSync(test.info().outputPath('reader.json'), record);
  console.log(record);
  expect(timeline.map(({ state }) => state)).toEqual(['no conversation', 'skeleton', 'no conversation']);

  await row(page, thread).click();
  await expect(viewer(page).getByText(thread.body(2))).toBeVisible();
  await shot(page, 'recovered');
});
