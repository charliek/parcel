import { expect, type FrameLocator, type Locator, type Page, test } from '@playwright/test';
import { createParcelViaApi, deleteParcelViaApi, parcelFrame, uniq } from './helpers';

let parcelId = '';

test.afterEach(async ({ request }) => {
  if (parcelId) await deleteParcelViaApi(request, parcelId);
  parcelId = '';
});

/**
 * Whether a comment pin covers the spot just above-right of the centre of
 * `target` (where a pin for a click at its centre is drawn). Pins live in a
 * closed shadow root, so the check asks the frame's own hit-testing, which
 * reports the overlay's host element for a point over a pin.
 */
async function pinAtCenter(target: Locator): Promise<boolean> {
  return target.evaluate((el) => {
    const r = el.getBoundingClientRect();
    const hit = document.elementFromPoint(r.left + r.width / 2 + 10, r.top + r.height / 2 - 10);
    return hit?.localName === 'parcel-overlay';
  });
}

function commentButton(page: Page): Locator {
  return page.getByRole('banner').getByRole('button', { name: /^Comment/ });
}

async function openParcel(page: Page, marker: string): Promise<FrameLocator> {
  await page.goto(`/p/${parcelId}`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: marker })).toBeVisible();
  return frame;
}

test('comment on a spot in the page, see its pin, then resolve it', async ({ page, request }) => {
  const marker = uniq('comment-target');
  parcelId = await createParcelViaApi(request, { title: uniq('e2e-comments'), marker });
  const frame = await openParcel(page, marker);
  const heading = frame.locator('#marker');

  await commentButton(page).click();
  await expect(commentButton(page)).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByText('Pick a spot on the page to comment on it.')).toBeVisible();
  const panel = page.getByRole('complementary', { name: 'Comments' });
  await expect(panel.getByText('No open comments.')).toBeVisible();

  // A click in the page picks the spot (and doesn't reach the page itself).
  await heading.click();
  const composer = panel.getByRole('textbox', { name: 'Comment on this spot' });
  await expect(composer).toBeFocused();
  await expect(panel.getByText(`Commenting on “${marker}”`)).toBeVisible();
  await expect.poll(() => pinAtCenter(heading)).toBe(true); // the draft pin

  await composer.fill('Make this heading bigger.\nAnd bolder.');
  await panel.getByRole('button', { name: 'Comment', exact: true }).click();

  const thread = panel.getByRole('article', { name: /^Comment #1 by / });
  await expect(thread).toBeVisible();
  await expect(thread).toBeFocused();
  await expect(thread).toContainText('Make this heading bigger.');
  await expect(thread).toContainText('You');
  await expect(thread).toContainText('Owner');
  await expect(thread.getByRole('button', { name: /^Show on the page/ })).toContainText(marker);
  await expect(composer).toHaveCount(0); // back to the general composer
  await expect(panel.getByRole('textbox', { name: 'Add a comment' })).toHaveValue('');
  await expect(commentButton(page)).toHaveAccessibleName('Comment (1 open)');
  await expect.poll(() => pinAtCenter(heading)).toBe(true); // the thread's pin

  // Clicking the pin selects its thread.
  await panel.getByRole('textbox', { name: 'Add a comment' }).focus();
  const box = await heading.boundingBox();
  if (!box) throw new Error('heading has no box');
  await page.mouse.click(box.x + box.width / 2 + 10, box.y + box.height / 2 - 10);
  await expect(thread).toBeFocused();

  // Resolve: it leaves the open list and its pin goes with it.
  await thread.getByRole('button', { name: 'Resolve #1' }).click();
  await expect(page.getByText('Resolved #1')).toBeVisible();
  await expect(thread).toHaveCount(0);
  await expect(commentButton(page)).toHaveAccessibleName('Comment');
  await expect.poll(() => pinAtCenter(heading)).toBe(false);

  // Showing resolved comments brings both back.
  await panel.getByRole('button', { name: /^Resolved/ }).click();
  await expect(thread).toContainText('Resolved');
  await expect.poll(() => pinAtCenter(heading)).toBe(true);

  // Stored with its anchor and the version it was made on.
  const response = await request.get(`/api/v1/parcels/${parcelId}/comments?status=all`);
  const body = await response.json();
  expect(body.open).toBe(0);
  expect(body.comments).toHaveLength(1);
  expect(body.comments[0]).toMatchObject({
    number: 1,
    version: 1,
    body: 'Make this heading bigger.\nAnd bolder.',
    resolved: true,
    via: 'web',
    anchor: { path: 'index.html', text: marker },
  });
  expect(body.comments[0].anchor.selector).toContain('h1');
});

test('reply, edit and delete, with the keyboard shortcuts', async ({ page, request }) => {
  const marker = uniq('comment-thread');
  parcelId = await createParcelViaApi(request, { title: uniq('e2e-thread'), marker });
  // A general comment that arrives through the API, as one from the CLI would.
  const created = await request.post(`/api/v1/parcels/${parcelId}/comments`, {
    data: { body: '<b>not bold</b> general note' },
  });
  expect(created.status()).toBe(201);

  await openParcel(page, marker);
  await expect(commentButton(page)).toHaveAccessibleName('Comment (1 open)');

  // `c` toggles comments; Escape leaves comment mode but keeps the panel.
  await page.keyboard.press('c');
  const panel = page.getByRole('complementary', { name: 'Comments' });
  const thread = panel.getByRole('article', { name: /^Comment #1 by / });
  // Bodies are text, never HTML.
  await expect(thread).toContainText('<b>not bold</b> general note');
  await expect(thread.locator('b')).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(page.getByText('Pick a spot on the page to comment on it.')).toHaveCount(0);
  await expect(panel).toBeVisible();

  await thread.getByRole('button', { name: 'Reply to #1' }).click();
  const reply = thread.getByRole('textbox', { name: 'Reply to #1' });
  await reply.fill('A reply');
  await reply.press('Control+Enter');
  const replies = thread.getByRole('list', { name: 'Replies to #1' });
  await expect(replies).toContainText('A reply');

  await thread.getByRole('button', { name: 'Comment #2 options' }).click();
  await page.getByRole('menuitem', { name: 'Edit' }).click();
  const edit = thread.getByRole('textbox', { name: 'Edit comment #2' });
  await edit.fill('An edited reply');
  await thread.getByRole('button', { name: 'Save' }).click();
  await expect(replies).toContainText('An edited reply');
  await expect(replies).toContainText('edited');

  await thread.getByRole('button', { name: 'Comment #1 options' }).click();
  await page.getByRole('menuitem', { name: 'Delete…' }).click();
  const confirm = page.getByRole('alertdialog');
  await expect(confirm).toContainText('its 1 reply');
  await confirm.getByRole('button', { name: 'Delete comment' }).click();
  await expect(thread).toHaveCount(0);
  await expect(panel.getByText('No open comments.')).toBeVisible();

  await page.keyboard.press('c');
  await expect(panel).toHaveCount(0);
});
