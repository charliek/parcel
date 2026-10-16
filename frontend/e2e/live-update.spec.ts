import { expect, type Page, test } from '@playwright/test';
import {
  createParcelViaApi,
  deleteParcelViaApi,
  htmlPage,
  parcelFrame,
  publishVersionViaApi,
  uniq,
} from './helpers';

// The viewer polls every 5 s; allow a few polls.
const POLL_TIMEOUT = 15_000;

let parcelId = '';

test.afterEach(async ({ request }) => {
  if (parcelId) await deleteParcelViaApi(request, parcelId);
  parcelId = '';
});

async function turnOffAutoReload(page: Page) {
  await page.getByRole('button', { name: 'More actions' }).click();
  const toggle = page.getByRole('menuitemcheckbox', {
    name: 'Reload when a new version is published',
  });
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  await toggle.click();
}

test('the viewer shows a newly published version without a manual reload', async ({
  page,
  request,
}) => {
  const v1 = uniq('live-v1');
  const v2 = uniq('live-v2');
  parcelId = await createParcelViaApi(request, { title: uniq('e2e-live'), marker: v1 });

  await page.goto(`/p/${parcelId}`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: v1 })).toBeVisible();

  expect(await publishVersionViaApi(request, parcelId, htmlPage(v2))).toBe(2);

  await expect(frame.getByRole('heading', { name: v2 })).toBeVisible({ timeout: POLL_TIMEOUT });
  await expect(page.getByText('Updated to v2')).toBeVisible();
  // The version menu caught up too.
  await expect(page.getByRole('button', { name: /^Version 2/ })).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}$`));
});

test('with auto-reload off, a banner offers the new version', async ({ page, request }) => {
  const v1 = uniq('offer-v1');
  const v2 = uniq('offer-v2');
  parcelId = await createParcelViaApi(request, { title: uniq('e2e-offer'), marker: v1 });

  await page.goto(`/p/${parcelId}`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: v1 })).toBeVisible();
  await turnOffAutoReload(page);

  await publishVersionViaApi(request, parcelId, htmlPage(v2));
  const banner = page.getByRole('status').filter({ hasText: 'v2 is available' });
  await expect(banner).toBeVisible({ timeout: POLL_TIMEOUT });
  // The frame stays put until asked.
  await expect(frame.getByRole('heading', { name: v1 })).toBeVisible();

  await banner.getByRole('button', { name: 'Reload' }).click();
  await expect(frame.getByRole('heading', { name: v2 })).toBeVisible();
  await expect(banner).toHaveCount(0);
});

test('a viewer pinned to a version stays on it and learns the new number', async ({
  page,
  request,
}) => {
  const v1 = uniq('pinned-v1');
  parcelId = await createParcelViaApi(request, { title: uniq('e2e-pinned'), marker: v1 });

  await page.goto(`/p/${parcelId}/v/1`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: v1 })).toBeVisible();

  await publishVersionViaApi(request, parcelId, htmlPage(uniq('pinned-v2')));
  await expect(page.getByText('The current version is v2.')).toBeVisible({
    timeout: POLL_TIMEOUT,
  });
  await expect(frame.getByRole('heading', { name: v1 })).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}/v/1$`));
});

test('a half-written comment holds off the reload', async ({ page, request }) => {
  const v1 = uniq('draft-v1');
  const v2 = uniq('draft-v2');
  parcelId = await createParcelViaApi(request, { title: uniq('e2e-draft'), marker: v1 });

  await page.goto(`/p/${parcelId}`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: v1 })).toBeVisible();
  await page
    .getByRole('banner')
    .getByRole('button', { name: /^Comment/ })
    .click();
  const panel = page.getByRole('complementary', { name: 'Comments' });
  await panel.getByRole('textbox', { name: 'Add a comment' }).fill('Thinking…');

  await publishVersionViaApi(request, parcelId, htmlPage(v2));
  const banner = page.getByRole('status').filter({ hasText: 'v2 is available' });
  await expect(banner).toBeVisible({ timeout: POLL_TIMEOUT });
  await expect(banner).toContainText('finished your comment');
  await expect(frame.getByRole('heading', { name: v1 })).toBeVisible();

  // Once the comment is sent, auto-reload (on by default) catches up.
  await panel.getByRole('button', { name: 'Comment', exact: true }).click();
  await expect(panel.getByRole('article', { name: /^Comment #1 by / })).toBeVisible();
  await expect(frame.getByRole('heading', { name: v2 })).toBeVisible({ timeout: POLL_TIMEOUT });
  await expect(banner).toHaveCount(0);
});
