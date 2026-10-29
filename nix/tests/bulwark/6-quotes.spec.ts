import { expect, test } from '@playwright/test';
import { Thread, row, shot, signIn, viewer } from './mail';

// a reply as Bulwark writes one, quoting a mail whose own quote is a bare <blockquote>
const reply = (body: string) =>
  `<p>${body}</p><div data-quoted-html="" style="border-left:2px solid #c5c5c5;padding-left:12px">` +
  `<p>the reply before</p><blockquote dir="auto"><p>the first mail</p></blockquote></div>`;

test('a bare quote inside a reply draws the bar its reply quote draws', async ({ page }) => {
  const thread = await Thread.of(2, reply);
  await thread.markRead(1);
  await signIn(page);

  await row(page, thread).click();
  const body = viewer(page).frameLocator('iframe').first();
  await expect(body.getByText('the first mail')).toBeVisible();
  await expect(body.locator('blockquote')).toHaveCSS('border-left-width', '2px');
  await shot(page, 'nested-quotes');
});
