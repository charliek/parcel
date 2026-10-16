import { expect, type Locator, type Page, test } from '@playwright/test';
import {
  createParcelViaApi,
  deleteParcelViaApi,
  parcelFrame,
  patchJson,
  stubServerInfo,
  uniq,
} from './helpers';

let parcelId = '';

test.afterEach(async ({ request }) => {
  if (parcelId) await deleteParcelViaApi(request, parcelId);
  parcelId = '';
});

async function openShareDialog(page: Page, marker: string): Promise<Locator> {
  await page.goto(`/p/${parcelId}`);
  await expect(parcelFrame(page).getByRole('heading', { name: marker })).toBeVisible();
  await page.getByRole('banner').getByRole('button', { name: 'Share', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: /^Share “/ });
  await expect(dialog).toBeVisible();
  return dialog;
}

/** A radio card's label; the radio itself is visually hidden. */
function accessCard(dialog: Locator, label: string): Locator {
  return dialog.locator('label').filter({ hasText: label });
}

test('share with people by email, change their role, and remove them', async ({
  page,
  request,
}) => {
  const marker = uniq('share');
  parcelId = await createParcelViaApi(request, { title: uniq('e2e-share'), marker });
  const dialog = await openShareDialog(page, marker);

  const people = dialog.getByRole('list', { name: 'People with access' });
  await expect(people).toContainText('E2E User (you)');
  await expect(people).toContainText('Owner');
  await expect(dialog.getByText('Not shared with anyone yet')).toBeVisible();

  // The dialog opens on the email field.
  const email = dialog.getByRole('textbox', { name: 'Add people by email' });
  await expect(email).toBeFocused();
  const share = dialog.getByRole('button', { name: 'Share', exact: true });
  await expect(share).toBeDisabled();

  // The server's reasons show inline, on the field.
  await email.fill('not-an-email');
  await share.click();
  const error = dialog.getByRole('alert');
  await expect(error).toContainText('is not an email address');
  await expect(email).toHaveAttribute('aria-invalid', 'true');
  await expect(email).toBeFocused();

  await email.fill('E2E@Example.com');
  await share.click();
  await expect(error).toContainText('owners always have access');

  // Typing again clears the error.
  const ada = `${uniq('ada')}@example.com`;
  await email.fill(ada.toUpperCase());
  await expect(error).toHaveCount(0);
  await dialog.getByLabel('Role for the person you add').selectOption('commenter');
  await share.click();

  await expect(email).toHaveValue('');
  await expect(email).toBeFocused();
  const row = people.getByTestId('share-row').filter({ hasText: ada });
  await expect(row).toContainText('Hasn’t signed in yet');
  const role = row.getByRole('combobox', { name: `Role for ${ada}` });
  await expect(role).toHaveValue('commenter');
  await expect(dialog.getByText('Not shared with anyone yet')).toHaveCount(0);

  // Change their role.
  await role.selectOption('viewer');
  await expect(page.getByText(`${ada}: Can view`)).toBeVisible();
  const listed = async () =>
    (await (await request.get(`/api/v1/parcels/${parcelId}/shares`)).json()).shares;
  expect(await listed()).toEqual([expect.objectContaining({ email: ada, role: 'viewer' })]);

  // It's still there after closing and reopening.
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await page.getByRole('banner').getByRole('button', { name: 'Share', exact: true }).click();
  await expect(row.getByRole('combobox', { name: `Role for ${ada}` })).toHaveValue('viewer');

  // Remove them: the list empties and focus goes back to the field.
  await row.getByRole('button', { name: `Remove ${ada}` }).click();
  await expect(row).toHaveCount(0);
  await expect(email).toBeFocused();
  expect(await listed()).toEqual([]);
});

test('general access in the share dialog, and the link', async ({ page, request, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  const marker = uniq('access');
  parcelId = await createParcelViaApi(request, { title: uniq('e2e-access'), marker });
  const dialog = await openShareDialog(page, marker);
  const header = page.getByRole('banner');

  await expect(dialog.getByRole('radio', { name: 'Private' })).toBeChecked();
  await accessCard(dialog, 'Everyone signed in').click();
  await expect(dialog.getByRole('radio', { name: 'Everyone signed in' })).toBeChecked();
  await expect(page.getByText('Parcel is now open to everyone signed in')).toBeVisible();
  let body = await (await request.get(`/api/v1/parcels/${parcelId}`)).json();
  expect(body.parcel.visibility).toBe('internal');

  // With general access open, the people list says roles don't narrow it.
  await dialog.getByRole('textbox', { name: 'Add people by email' }).fill('bo@example.com');
  await dialog.getByRole('button', { name: 'Share', exact: true }).click();
  await expect(dialog.getByText('everyone here can comment')).toBeVisible();

  await accessCard(dialog, 'Anyone with the link').click();
  await expect(dialog.getByRole('radio', { name: 'Anyone with the link' })).toBeChecked();
  await expect(page.getByText('Parcel is now public')).toBeVisible();
  body = await (await request.get(`/api/v1/parcels/${parcelId}`)).json();
  expect(body.parcel.visibility).toBe('public');

  await expect(dialog.getByRole('textbox', { name: 'Link' })).toHaveValue(
    new RegExp(`/p/${parcelId}$`),
  );
  await dialog.getByRole('button', { name: 'Copy link' }).click();
  await expect(page.getByText('Link copied')).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toMatch(
    new RegExp(`/p/${parcelId}$`),
  );

  // The header's badge followed (the page behind a modal is hidden from roles).
  await page.keyboard.press('Escape');
  await expect(header.getByText('Public', { exact: true })).toBeVisible();
});

test('with public links turned off, the option says why', async ({ page, request }) => {
  await stubServerInfo(page, { allowPublic: false });
  const marker = uniq('no-public');
  parcelId = await createParcelViaApi(request, { title: uniq('e2e-no-public'), marker });
  const dialog = await openShareDialog(page, marker);

  const publicOption = dialog.getByRole('radio', { name: 'Anyone with the link' });
  await expect(publicOption).toBeDisabled();
  await expect(publicOption).toHaveAccessibleDescription(
    'Public links are turned off on this server.',
  );
  await expect(dialog.getByRole('radio', { name: 'Everyone signed in' })).toBeEnabled();

  // The menu leaves it out.
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'More actions' }).click();
  await expect(page.getByRole('menuitemradio', { name: 'Everyone signed in' })).toBeVisible();
  await expect(page.getByRole('menuitemradio', { name: 'Anyone with the link' })).toHaveCount(0);
  await page.keyboard.press('Escape');

  // And so does publishing a new parcel.
  await page.goto('/new');
  await expect(page.getByRole('radio', { name: 'Anyone with the link' })).toBeDisabled();
  await expect(page.getByRole('radio', { name: 'Private' })).toBeChecked();
});

test('shared with me lists other people’s parcels with owner and role', async ({
  page,
  request,
}) => {
  // The dev user is alone on the e2e stack, so the real list is empty.
  await page.goto('/?tab=shared');
  await expect(page.getByRole('heading', { name: 'Shared with me', level: 1 })).toBeVisible();
  await expect(page.getByRole('tab', { name: /Shared with me/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await expect(page.getByRole('heading', { name: 'Nothing shared with you yet' })).toBeVisible();

  // Someone else's parcel, as the list would show it.
  const marker = uniq('shared');
  const title = uniq('e2e-shared');
  parcelId = await createParcelViaApi(request, { title, marker });
  const summary = (await (await request.get(`/api/v1/parcels/${parcelId}`)).json()).parcel;
  await patchJson(page, /\/api\/v1\/shared(\?.*)?$/, () => ({
    parcels: [
      {
        parcel: summary,
        owner_name: 'Ada Owner',
        role: 'commenter',
        shared_at: summary.created_at,
      },
    ],
    total: 1,
  }));

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'My parcels', level: 1 })).toBeVisible();
  await expect(page.getByTestId('shared-count')).toHaveText('1');
  await page.getByRole('tab', { name: /Shared with me/ }).click();
  await expect(page).toHaveURL(/\?tab=shared$/);
  const card = page.getByTestId('parcel-card').filter({ hasText: title });
  await expect(card).toContainText('Ada Owner · Can comment');
  await card.click();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}$`));
  await expect(parcelFrame(page).getByRole('heading', { name: marker })).toBeVisible();
});
