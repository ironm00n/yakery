import { expect, test } from '@playwright/test';
import { Thread, folded, row, shot, signIn, unfolded, viewer } from './mail';

test('a thread opens as one conversation, and unfolding a message loads its body', async ({ page }) => {
  const thread = await Thread.of(2);
  await thread.markRead(1);
  await signIn(page);

  await row(page, thread).click();
  await expect(viewer(page).getByText(thread.body(2))).toBeVisible();
  await expect(folded(page)).toHaveCount(1);
  await shot(page, 'open');

  await folded(page).click();
  await expect(unfolded(page).getByText(thread.body(1))).toBeVisible();
  await shot(page, 'unfolded');
});
