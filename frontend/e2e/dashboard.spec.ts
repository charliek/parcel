import { expect, test } from '@playwright/test';
import { createParcelViaApi, deleteParcelViaApi, uniq } from './helpers';

test('dashboard loads for the dev user', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'My parcels', level: 1 })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Account menu' })).toBeVisible();
  await expect(page.getByRole('searchbox', { name: 'Search parcels' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'New parcel' }).first()).toBeVisible();
});

test('search filters parcels server-side', async ({ page, request }) => {
  const title = uniq('e2e-search');
  const id = await createParcelViaApi(request, { title, marker: title });
  try {
    await page.goto('/');
    const search = page.getByRole('searchbox', { name: 'Search parcels' });
    await search.fill(title);
    await expect(page).toHaveURL(new RegExp(`\\?q=${title}`));
    await expect(page.getByTestId('parcel-card').filter({ hasText: title })).toBeVisible();
    await expect(page.getByTestId('parcel-card')).toHaveCount(1);

    await search.fill(uniq('nothing-matches'));
    await expect(page.getByRole('heading', { name: 'No matching parcels' })).toBeVisible();

    // Clicking a card opens the viewer.
    await search.fill(title);
    await page.getByTestId('parcel-card').filter({ hasText: title }).click();
    await expect(page).toHaveURL(new RegExp(`/p/${id}$`));
  } finally {
    await deleteParcelViaApi(request, id);
  }
});

test('account menu links to tokens and CLI setup', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Account menu' }).click();
  await page.getByRole('menuitem', { name: 'CLI setup' }).click();
  await expect(page.getByRole('heading', { name: 'CLI setup', level: 1 })).toBeVisible();
  await expect(page.getByText('parcel skill install')).toBeVisible();
});

test('unknown routes and parcels show not-found pages', async ({ page }) => {
  await page.goto('/definitely-not-a-page');
  await expect(page.getByRole('heading', { name: "This page doesn't exist" })).toBeVisible();

  // Well-formed but (almost certainly) nonexistent id; dev mode is signed in.
  await page.goto('/p/zzzzzzzzzzzz');
  await expect(page.getByRole('heading', { name: 'Parcel not found' })).toBeVisible();

  // Malformed id: no request, straight to not found.
  await page.goto('/p/not-an-id');
  await expect(page.getByRole('heading', { name: 'Parcel not found' })).toBeVisible();
});
