import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';
import {
  deleteParcelViaApi,
  htmlPage,
  PARCEL_URL,
  parcelFrame,
  parcelIdFrom,
  uniq,
  viewerAction,
} from './helpers';

// One parcel walks through the whole lifecycle, so run these in order.
test.describe.configure({ mode: 'serial' });

const title = uniq('e2e-parcel');
const markerV1 = uniq('first');
const markerV2 = uniq('second');
let parcelId = '';

test.afterAll(async ({ request }) => {
  if (parcelId) await deleteParcelViaApi(request, parcelId);
});

test('create a parcel by uploading files and see it in the sandbox', async ({ page }) => {
  await page.goto('/new');
  await expect(page.getByRole('heading', { name: 'New parcel', level: 1 })).toBeVisible();

  await page.getByTestId('file-input').setInputFiles([
    {
      name: 'index.html',
      mimeType: 'text/html',
      buffer: htmlPage(markerV1, { stylesheet: 'style.css' }),
    },
    {
      name: 'style.css',
      mimeType: 'text/css',
      buffer: Buffer.from('#marker { color: rgb(1, 2, 3); }'),
    },
  ]);
  const files = page.getByRole('list', { name: 'Selected files' });
  await expect(files.getByText('index.html')).toBeVisible();
  await expect(files.getByText('style.css')).toBeVisible();
  await expect(files.getByText('Entry')).toBeVisible();

  await page.getByLabel('Title').fill(title);
  await page.getByLabel('Description').fill('Created by the Playwright suite');
  await page.getByRole('button', { name: 'Publish parcel' }).click();

  await expect(page).toHaveURL(PARCEL_URL);
  parcelId = parcelIdFrom(page);
  await expect(page.getByRole('heading', { name: title, level: 1 })).toBeVisible();

  // The iframe is sandboxed without same-origin access, and renders the page
  // with its relative stylesheet.
  const iframe = page.locator('iframe[data-testid="parcel-frame"]');
  const sandbox = (await iframe.getAttribute('sandbox')) ?? '';
  expect(sandbox).toContain('allow-scripts');
  expect(sandbox).not.toContain('allow-same-origin');
  await expect(iframe).toHaveAttribute('referrerpolicy', 'no-referrer');

  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: markerV1 })).toBeVisible();
  await expect(frame.locator('#marker')).toHaveCSS('color', 'rgb(1, 2, 3)');
});

test('the parcel is listed on the dashboard', async ({ page }) => {
  await page.goto('/');
  const card = page.getByTestId('parcel-card').filter({ hasText: title });
  await expect(card).toBeVisible();
  await expect(card).toContainText('Private');
  await expect(card).toContainText('v1');
});

test('toggle public and private', async ({ page }) => {
  await page.goto(`/p/${parcelId}`);
  const header = page.getByRole('banner');
  await expect(header.getByText('Private', { exact: true })).toBeVisible();

  await viewerAction(page, 'Make public');
  await expect(header.getByText('Public', { exact: true })).toBeVisible();
  // The frame is not reloaded by a metadata change.
  await expect(parcelFrame(page).getByRole('heading', { name: markerV1 })).toBeVisible();

  await viewerAction(page, 'Make private');
  await expect(header.getByText('Private', { exact: true })).toBeVisible();
});

test('details panel lists files and history', async ({ page }) => {
  await page.goto(`/p/${parcelId}`);
  await page.getByRole('button', { name: 'Show details' }).click();
  const panel = page.getByRole('complementary', { name: 'Parcel details' });
  await expect(panel.getByRole('list', { name: 'Files' })).toContainText('index.html');
  await expect(panel.getByRole('list', { name: 'Files' })).toContainText('style.css');
  await expect(panel.getByText('Created by the Playwright suite')).toBeVisible();
  await page.getByRole('button', { name: 'Close details' }).click();
  await expect(panel).toHaveCount(0);
});

test('upload a new version, then browse back to the old one', async ({ page }) => {
  await page.goto(`/p/${parcelId}`);
  await viewerAction(page, 'Upload new version…');
  const dialog = page.getByRole('dialog', { name: 'Upload a new version' });
  await dialog.getByTestId('file-input').setInputFiles([
    {
      name: 'index.html',
      mimeType: 'text/html',
      buffer: htmlPage(markerV2, { stylesheet: 'style.css' }),
    },
  ]);
  await dialog.getByLabel('Version note').fill('Second draft');
  await dialog.getByRole('button', { name: 'Publish version' }).click();
  await expect(dialog).toHaveCount(0);

  // Merge mode kept style.css; the frame now shows v2.
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: markerV2 })).toBeVisible();
  await expect(frame.locator('#marker')).toHaveCSS('color', 'rgb(1, 2, 3)');

  await page.getByRole('button', { name: /^Version 2/ }).click();
  await page.getByRole('menuitemradio', { name: /v1/ }).click();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}/v/1$`));
  await expect(page.getByText(/Viewing v1 from/)).toBeVisible();
  await expect(frame.getByRole('heading', { name: markerV1 })).toBeVisible();

  await page.getByRole('link', { name: 'View current' }).click();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}$`));
  await expect(frame.getByRole('heading', { name: markerV2 })).toBeVisible();
});

test('restore an old version as a new one', async ({ page }) => {
  await page.goto(`/p/${parcelId}/v/1`);
  await page.getByRole('button', { name: 'Restore this version' }).click();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}$`));
  await expect(page.getByRole('button', { name: /^Version 3/ })).toBeVisible();
  await expect(parcelFrame(page).getByRole('heading', { name: markerV1 })).toBeVisible();
});

test('edit title and description', async ({ page }) => {
  await page.goto(`/p/${parcelId}`);
  await viewerAction(page, 'Edit details…');
  const dialog = page.getByRole('dialog', { name: 'Edit details' });
  await dialog.getByLabel('Title').fill(`${title} (edited)`);
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByRole('heading', { name: `${title} (edited)`, level: 1 })).toBeVisible();
});

test('delete the parcel', async ({ page }) => {
  await page.goto(`/p/${parcelId}`);
  await viewerAction(page, 'Delete parcel…');
  const confirm = page.getByRole('alertdialog');
  await expect(confirm).toContainText('permanently deletes');
  await confirm.getByRole('button', { name: 'Delete parcel' }).click();

  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole('heading', { name: 'My parcels', level: 1 })).toBeVisible();
  await expect(page.getByTestId('parcel-card').filter({ hasText: title })).toHaveCount(0);

  await page.goto(`/p/${parcelId}`);
  await expect(page.getByRole('heading', { name: 'Parcel not found' })).toBeVisible();
  parcelId = '';
});

test('folder upload publishes paths relative to the folder', async ({ page, request }) => {
  const marker = uniq('folder');
  const dir = mkdtempSync(join(tmpdir(), 'parcel-e2e-'));
  const site = join(dir, 'site');
  mkdirSync(join(site, 'assets'), { recursive: true });
  writeFileSync(join(site, 'index.html'), htmlPage(marker, { stylesheet: 'assets/site.css' }));
  writeFileSync(join(site, 'assets', 'site.css'), '#marker { color: rgb(4, 5, 6); }');
  writeFileSync(join(site, '.DS_Store'), 'junk');
  let id = '';
  try {
    await page.goto('/new');
    await page.getByTestId('folder-input').setInputFiles(site);
    const files = page.getByRole('list', { name: 'Selected files' });
    // The top folder ("site/") is stripped and hidden files are skipped.
    await expect(files.getByText('index.html', { exact: true })).toBeVisible();
    await expect(files.getByText('assets/site.css', { exact: true })).toBeVisible();
    await expect(files.getByText('.DS_Store')).toHaveCount(0);

    await page.getByRole('button', { name: 'Publish parcel' }).click();
    await expect(page).toHaveURL(PARCEL_URL);
    id = parcelIdFrom(page);
    await expect(parcelFrame(page).locator('#marker')).toHaveCSS('color', 'rgb(4, 5, 6)');
  } finally {
    rmSync(dir, { recursive: true, force: true });
    if (id) await deleteParcelViaApi(request, id);
  }
});
