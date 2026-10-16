import { expect, type Page, test } from '@playwright/test';
import {
  createParcelViaApi,
  deleteParcelViaApi,
  parcelFrame,
  patchJson,
  stubServerInfo,
  stubSignedOut,
  uniq,
} from './helpers';

// The e2e stack has one (dev) user, the owner of everything. These specs
// rewrite the grant and the parcel detail to see the viewer as someone the
// parcel is shared with; the content, comments and polls are real.

let parcelId = '';
let marker = '';

test.beforeEach(async ({ request }) => {
  marker = uniq('access-target');
  parcelId = await createParcelViaApi(request, { title: uniq('e2e-access'), marker });
  const response = await request.post(`/api/v1/parcels/${parcelId}/comments`, {
    data: { body: 'The owner left a note.', version: 1 },
  });
  expect(response.status(), await response.text()).toBe(201);
});

test.afterEach(async ({ request }) => {
  if (parcelId) await deleteParcelViaApi(request, parcelId);
  parcelId = '';
});

/** See the parcel as `access`, shared by Ada (no owner name when signed out). */
async function viewAs(page: Page, access: 'viewer' | 'commenter' | 'public') {
  const asCaller = (body: any) => ({
    ...body,
    access,
    is_owner: false,
    owner_name: access === 'public' ? undefined : 'Ada Owner',
  });
  await patchJson(page, new RegExp(`/api/v1/parcels/${parcelId}/view(\\?.*)?$`), asCaller);
  await patchJson(page, new RegExp(`/api/v1/parcels/${parcelId}$`), asCaller);
}

function commentButton(page: Page) {
  return page.getByRole('banner').getByRole('button', { name: /^Comment/ });
}

async function expectNoOwnerControls(page: Page) {
  const header = page.getByRole('banner');
  await expect(header.getByRole('button', { name: 'Share', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'More actions' }).click();
  const menu = page.getByRole('menu');
  await expect(menu.getByRole('menuitemcheckbox', { name: /Reload when/ })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: 'Edit details…' })).toHaveCount(0);
  await expect(menu.getByRole('menuitem', { name: 'Delete parcel…' })).toHaveCount(0);
  await expect(menu.getByRole('menuitemradio')).toHaveCount(0);
  await page.keyboard.press('Escape');
}

test('a viewer reads comments but cannot comment', async ({ page }) => {
  await viewAs(page, 'viewer');
  await page.goto(`/p/${parcelId}`);
  await expect(parcelFrame(page).getByRole('heading', { name: marker })).toBeVisible();

  await expect(page.getByTestId('shared-by')).toHaveText('Shared by Ada Owner');
  await expectNoOwnerControls(page);

  // The button opens the threads, not comment mode.
  await expect(commentButton(page)).toHaveAccessibleName('Comments (1 open)');
  await commentButton(page).click();
  const panel = page.getByRole('complementary', { name: 'Comments' });
  const thread = panel.getByRole('article', { name: /^Comment #1 by / });
  await expect(thread).toContainText('The owner left a note.');
  await expect(page.getByText('Pick a spot on the page to comment on it.')).toHaveCount(0);
  await expect(panel.getByTestId('view-only-note')).toHaveText(
    /You can view this parcel and its comments, but not comment\./,
  );
  await expect(panel.getByRole('textbox')).toHaveCount(0);
  await expect(panel.getByRole('button', { name: 'Comment on a spot in the page' })).toHaveCount(0);
  await expect(thread.getByRole('button', { name: 'Reply to #1' })).toHaveCount(0);
  await expect(thread.getByRole('button', { name: 'Resolve #1' })).toHaveCount(0);
  await expect(thread.getByRole('button', { name: 'Comment #1 options' })).toHaveCount(0);

  // A click in the page is just a click.
  await parcelFrame(page).locator('#marker').click();
  await expect(panel.getByText(/Commenting on/)).toHaveCount(0);

  // `c` closes and reopens the threads (once focus is back out of the frame).
  await panel.getByRole('heading', { name: 'Comments' }).click();
  await page.keyboard.press('c');
  await expect(panel).toHaveCount(0);
  await page.keyboard.press('c');
  await expect(panel).toBeVisible();

  // The details say whose it is and what the caller may do.
  await page.getByRole('button', { name: 'Show details' }).click();
  const details = page.getByRole('complementary', { name: 'Parcel details' });
  await expect(details.getByText('Ada Owner')).toBeVisible();
  await expect(details.getByText('Can view')).toBeVisible();
  await expect(details.getByRole('button', { name: 'Edit details' })).toHaveCount(0);
});

test('a commenter gets comment mode and the composer', async ({ page }) => {
  await viewAs(page, 'commenter');
  await page.goto(`/p/${parcelId}`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: marker })).toBeVisible();

  await expect(page.getByTestId('shared-by')).toHaveText('Shared by Ada Owner');
  await expectNoOwnerControls(page);

  await commentButton(page).click();
  await expect(page.getByText('Pick a spot on the page to comment on it.')).toBeVisible();
  const panel = page.getByRole('complementary', { name: 'Comments' });
  await expect(panel.getByTestId('view-only-note')).toHaveCount(0);
  await expect(panel.getByRole('button', { name: 'Reply to #1' })).toBeVisible();

  await frame.locator('#marker').click();
  const composer = panel.getByRole('textbox', { name: 'Comment on this spot' });
  await expect(composer).toBeFocused();
  await composer.fill('A commenter’s thought.');
  await panel.getByRole('button', { name: 'Comment', exact: true }).click();
  await expect(panel.getByRole('article', { name: /^Comment #2 by / })).toContainText(
    'A commenter’s thought.',
  );
});

test('signed out on a public parcel: no comments, and an offer to sign in', async ({ page }) => {
  await stubServerInfo(page, { providers: [{ id: 'slauth', label: 'slauth' }] });
  await stubSignedOut(page);
  await viewAs(page, 'public');
  await page.goto(`/p/${parcelId}`);
  await expect(parcelFrame(page).getByRole('heading', { name: marker })).toBeVisible();

  const header = page.getByRole('banner');
  await expect(page.getByTestId('shared-by')).toHaveCount(0);
  await expect(header.getByRole('button', { name: 'Sign in' })).toBeVisible();
  await expect(header.getByRole('button', { name: 'Share', exact: true })).toHaveCount(0);

  // The count of open comments isn't shown to signed-out visitors.
  await expect(commentButton(page)).toHaveAccessibleName('Comment');
  await commentButton(page).click();
  const prompt = page.getByRole('dialog', { name: 'Sign in to comment' });
  await expect(prompt).toBeVisible();
  await expect(page.getByRole('complementary', { name: 'Comments' })).toHaveCount(0);
  await prompt.getByRole('button', { name: 'Not now' }).click();
  await expect(prompt).toHaveCount(0);
});

test('an open viewer notices when the parcel goes away', async ({ page, request }) => {
  await page.goto(`/p/${parcelId}`);
  await expect(parcelFrame(page).getByText(marker)).toBeVisible();
  // Deleting (like unsharing, or making it private) turns the next poll
  // into a 404; the viewer drops the page instead of leaving it on screen.
  await deleteParcelViaApi(request, parcelId);
  parcelId = '';
  await expect(page.getByRole('heading', { name: 'Parcel not found' })).toBeVisible({
    timeout: 15_000,
  });
  await expect(parcelFrame(page).getByText(marker)).toHaveCount(0);
});
