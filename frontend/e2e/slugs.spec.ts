import { expect, test } from '@playwright/test';
import {
  deleteParcelViaApi,
  htmlPage,
  myHandle,
  parcelFrame,
  patchParcelViaApi,
  publishViaApi,
  uniq,
  uniqTag,
} from './helpers';

const ids: string[] = [];
let restoreHandle: string | null = null;

test.afterEach(async ({ request }) => {
  for (const id of ids.splice(0)) await deleteParcelViaApi(request, id);
  if (restoreHandle) {
    // Taking back your own old handle is always allowed.
    await request.patch('/api/v1/me', { data: { handle: restoreHandle } });
    restoreHandle = null;
  }
});

async function slugged(request: Parameters<typeof publishViaApi>[0], marker: string, slug: string) {
  const result = await publishViaApi(request, {
    metadata: { title: marker, slug },
    files: [
      { path: 'index.html', body: htmlPage(marker) },
      { path: 'more.html', body: htmlPage(`${marker} more`) },
    ],
  });
  ids.push(result.parcel.id);
  return result.parcel;
}

test('/@handle/slug opens the viewer, and a renamed slug redirects', async ({ page, request }) => {
  const handle = await myHandle(request);
  const marker = uniq('e2e-slug');
  const first = `${uniqTag('first')}/plan`;
  const parcel = await slugged(request, marker, first);
  expect(parcel.canonical_url).toMatch(new RegExp(`/@${handle}/${first}$`));

  await page.goto(`/@${handle}/${first}`);
  await expect(parcelFrame(page).getByRole('heading', { name: marker, exact: true })).toBeVisible();
  // Copy link copies the clean URL.
  await page.getByRole('button', { name: 'Show details' }).click();
  const details = page.getByRole('complementary', { name: 'Parcel details' });
  await expect(details.getByTestId('canonical-link')).toHaveText(`/@${handle}/${first}`);

  // The Pages menu keeps the clean address.
  await page.getByTestId('pages-menu').click();
  await page.getByRole('menuitemradio', { name: /more\.html/ }).click();
  await expect(page).toHaveURL(new RegExp(`/@${handle}/${first}\\?page=more\\.html$`));
  await expect(parcelFrame(page).getByRole('heading', { name: `${marker} more` })).toBeVisible();

  // Rename the slug: the old address redirects, keeping page and fragment.
  const second = `${uniqTag('second')}/plan`;
  await patchParcelViaApi(request, parcel.id, { slug: second });
  // A fresh load (the app would otherwise reuse what it resolved a moment ago).
  await page.goto('about:blank');
  await page.goto(`/@${handle}/${first}?page=more.html#top`);
  await expect(page).toHaveURL(new RegExp(`/@${handle}/${second}\\?page=more\\.html#top$`));
  await expect(parcelFrame(page).getByRole('heading', { name: `${marker} more` })).toBeVisible();

  // A version pin on a clean URL is `?v=`.
  await page.goto(`/@${handle}/${second}?v=1`);
  await expect(
    page.getByRole('button', { name: /^Version 1, choose another version/ }),
  ).toBeVisible();
  await expect(parcelFrame(page).getByRole('heading', { name: marker, exact: true })).toBeVisible();
});

test('the handle dialog changes the handle, and old links keep working', async ({
  page,
  request,
}) => {
  const original = await myHandle(request);
  const marker = uniq('e2e-handle');
  const slug = uniqTag('handled');
  await slugged(request, marker, slug);

  await page.goto('/');
  await page.getByRole('button', { name: 'Account menu' }).click();
  await expect(page.getByTestId('user-handle')).toHaveText(`@${original}`);
  await page.getByRole('menuitem', { name: 'Handle…' }).click();
  const dialog = page.getByRole('dialog', { name: 'Your handle' });
  const input = dialog.getByRole('textbox', { name: 'Handle' });
  await expect(input).toHaveValue(original);

  // Checked as you type.
  await input.fill('-bad-');
  await expect(dialog.getByTestId('handle-error')).toContainText(
    'starting and ending with a letter',
  );
  await expect(dialog.getByRole('button', { name: 'Save' })).toBeDisabled();

  const next = uniqTag('h').slice(0, 30);
  await input.fill(next.toUpperCase());
  await expect(dialog.getByTestId('handle-preview')).toHaveText(new RegExp(`/@${next}/your-slug$`));
  await expect(dialog).toContainText('Links with your old handle keep working');
  await dialog.getByRole('button', { name: 'Save' }).click();
  restoreHandle = original;
  await expect(dialog).toBeHidden();
  await expect(page.getByText(`Your handle is now @${next}`)).toBeVisible();
  expect(await myHandle(request)).toBe(next);

  // The old clean URL redirects to the new one.
  await page.goto(`/@${original}/${slug}`);
  await expect(page).toHaveURL(new RegExp(`/@${next}/${slug}$`));
  await expect(parcelFrame(page).getByRole('heading', { name: marker, exact: true })).toBeVisible();
});

test('a handle changed from the viewer updates its link', async ({ page, request }) => {
  const original = await myHandle(request);
  const marker = uniq('e2e-handle-viewer');
  const slug = uniqTag('viewed');
  await slugged(request, marker, slug);

  await page.goto(`/@${original}/${slug}`);
  await expect(parcelFrame(page).getByRole('heading', { name: marker, exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Show details' }).click();
  const link = page
    .getByRole('complementary', { name: 'Parcel details' })
    .getByTestId('canonical-link');
  await expect(link).toHaveText(`/@${original}/${slug}`);

  await page.getByRole('button', { name: 'Account menu' }).click();
  await page.getByRole('menuitem', { name: 'Handle…' }).click();
  const dialog = page.getByRole('dialog', { name: 'Your handle' });
  const next = uniqTag('v').slice(0, 30);
  await dialog.getByRole('textbox', { name: 'Handle' }).fill(next);
  await dialog.getByRole('button', { name: 'Save' }).click();
  restoreHandle = original;
  await expect(dialog).toBeHidden();

  // What is shown and what Copy link copies name the new handle.
  await expect(link).toHaveText(`/@${next}/${slug}`);
  await expect(link).toHaveAttribute('title', new RegExp(`/@${next}/${slug}$`));
});

test('a taken handle is refused inline', async ({ page }) => {
  // The dev user is alone on the stack; answer as the server does for a taken handle.
  await page.route('**/api/v1/me', async (route) => {
    if (route.request().method() !== 'PATCH') return route.fallback();
    await route.fulfill({
      status: 409,
      json: { error: { message: 'that handle is taken', type: 'Conflict' } },
    });
  });
  await page.goto('/');
  await page.getByRole('button', { name: 'Account menu' }).click();
  await page.getByRole('menuitem', { name: 'Handle…' }).click();
  const dialog = page.getByRole('dialog', { name: 'Your handle' });
  await dialog.getByRole('textbox', { name: 'Handle' }).fill('someone-else');
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(dialog.getByTestId('handle-error')).toHaveText('That handle is taken.');
  await expect(dialog).toBeVisible();
});
