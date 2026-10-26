import { expect, test } from '@playwright/test';
import { Thread, folded, row, shot, signIn, unfolded, viewer } from './mail';

test('an open thread takes in the mail delivered to it and the reply sent from it', async ({ page }) => {
  const thread = await Thread.of(2);
  await signIn(page);
  await row(page, thread).click();
  await expect(viewer(page).getByText(thread.body(2))).toBeVisible();

  await thread.add();
  await expect(unfolded(page).getByText(thread.body(3))).toBeVisible();
  await shot(page, 'delivered');

  const reply = `a quick reply to ${thread.subject}`;
  await viewer(page).getByPlaceholder('Write a quick reply...').fill(reply);
  await viewer(page).getByRole('button', { name: 'Send', exact: true }).click();
  await expect(folded(page).or(unfolded(page)).getByText(reply)).toBeVisible();
  await shot(page, 'replied');
});
