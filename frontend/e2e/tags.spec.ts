import { expect, type Page, test } from '@playwright/test';
import {
  deleteParcelViaApi,
  htmlPage,
  myHandle,
  parcelFrame,
  publishViaApi,
  uniq,
  uniqTag,
  viewerAction,
} from './helpers';

const ids: string[] = [];

test.afterEach(async ({ request }) => {
  for (const id of ids.splice(0)) await deleteParcelViaApi(request, id);
});

async function tagged(
  request: Parameters<typeof publishViaApi>[0],
  title: string,
  metadata: object,
): Promise<any> {
  const result = await publishViaApi(request, {
    metadata: { title, ...metadata },
    files: [{ path: 'index.html', body: htmlPage(title) }],
  });
  ids.push(result.parcel.id);
  return result.parcel;
}

async function openEdit(page: Page, id: string, marker: string) {
  await page.goto(`/p/${id}`);
  await expect(parcelFrame(page).getByRole('heading', { name: marker })).toBeVisible();
  await viewerAction(page, 'Edit details…');
  return page.getByRole('dialog', { name: 'Edit details' });
}

test('edit tags and the slug in the details dialog', async ({ page, request }) => {
  const known = uniqTag('known');
  const title = uniq('e2e-edit-tags');
  // Another parcel gives the vocabulary a tag (and a slug to collide with).
  const otherSlug = uniqTag('taken');
  await tagged(request, uniq('e2e-vocab'), { tags: [known], slug: otherSlug });
  const parcel = await tagged(request, title, {});
  const handle = await myHandle(request);

  const dialog = await openEdit(page, parcel.id, title);
  const tags = dialog.getByRole('combobox', { name: 'Tags' });
  const list = dialog.getByRole('listbox', { name: 'Tag suggestions' });

  // A new tag: shown as it will be saved, offered as "Create …".
  await tags.click();
  await tags.pressSequentially('Big Plan');
  await expect(dialog.getByText('Saved as “big-plan”.')).toBeVisible();
  await expect(list.getByRole('option')).toHaveCount(1);
  await expect(list.getByRole('option')).toContainText('Create big-plan');
  await tags.press('Enter');
  await expect(dialog.getByTestId('tag-input')).toContainText('big-plan');
  await expect(tags).toHaveValue('');
  await expect(tags).toBeFocused();

  // An existing tag comes with its count, before "Create …".
  await tags.pressSequentially(known.slice(0, -2));
  const first = list.getByRole('option').first();
  await expect(first).toContainText(known);
  await expect(first).toContainText('1 parcel');
  await expect(first).toHaveAttribute('aria-selected', 'true');
  await expect(list.getByRole('option').last()).toContainText('Create');
  // ↓ lands on the highlighted match, not past it to "Create …".
  await tags.press('ArrowDown');
  await expect(first).toHaveAttribute('aria-selected', 'true');
  // Escape closes the suggestions, not the dialog.
  await tags.press('Escape');
  await expect(list).toHaveCount(0);
  await expect(dialog).toBeVisible();
  await tags.press('ArrowDown');
  await tags.press('Enter');
  await expect(dialog.getByTestId('tag-input')).toContainText(known);

  // Backspace in the empty input removes the last chip.
  await tags.press('Backspace');
  await expect(dialog.getByTestId('tag-input')).not.toContainText(known);
  await tags.pressSequentially(known);
  await tags.press('Enter');

  // The slug: rules checked as you type, the clean URL previewed.
  const slug = dialog.getByRole('textbox', { name: 'Slug' });
  await slug.fill('p/oops');
  await expect(dialog.getByTestId('slug-error')).toContainText("may not be 'p', 'v'");
  await expect(dialog.getByRole('button', { name: 'Save' })).toBeDisabled();
  await slug.fill(otherSlug);
  await expect(dialog.getByTestId('slug-preview')).toHaveText(
    new RegExp(`/@${handle}/${otherSlug}$`),
  );
  // Taken by the other parcel: the server's 409, said inline.
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(dialog.getByTestId('slug-error')).toHaveText(
    'Already used by another of your parcels.',
  );

  const mine = uniqTag('mine');
  await slug.fill(`E2E/${mine}`);
  await expect(dialog.getByTestId('slug-preview')).toHaveText(
    new RegExp(`/@${handle}/e2e/${mine}$`),
  );
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(dialog).toBeHidden();

  const saved = (await (await request.get(`/api/v1/parcels/${parcel.id}`)).json()).parcel;
  expect(saved.tags).toEqual(['big-plan', known].sort());
  expect(saved.slug).toBe(`e2e/${mine}`);

  // The details panel shows them; tag chips link to the filtered dashboard.
  await page.getByRole('button', { name: 'Show details' }).click();
  const details = page.getByRole('complementary', { name: 'Parcel details' });
  await expect(details.getByTestId('canonical-link')).toHaveText(`/@${handle}/e2e/${mine}`);
  await expect(details.getByRole('link', { name: 'big-plan' })).toHaveAttribute(
    'href',
    '/?tag=big-plan',
  );

  // Clearing both sends `tags: []` and `slug: ""`.
  let patch: unknown = null;
  page.on('request', (r) => {
    if (r.method() === 'PATCH' && r.url().endsWith(`/api/v1/parcels/${parcel.id}`)) {
      patch = r.postDataJSON();
    }
  });
  await viewerAction(page, 'Edit details…');
  await tags.press('Backspace');
  await tags.press('Backspace');
  await slug.fill('');
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(dialog).toBeHidden();
  expect(patch).toEqual({ tags: [], slug: '' });
  const cleared = (await (await request.get(`/api/v1/parcels/${parcel.id}`)).json()).parcel;
  expect(cleared.tags).toEqual([]);
  expect(cleared.slug).toBeNull();
});

test('archive and unarchive from the viewer menu', async ({ page, request }) => {
  const title = uniq('e2e-archive');
  const parcel = await tagged(request, title, {});
  await page.goto(`/p/${parcel.id}`);
  await expect(parcelFrame(page).getByRole('heading', { name: title })).toBeVisible();
  await viewerAction(page, 'Archive');
  await expect(page.getByTestId('archived-badge')).toBeVisible();
  const archived = (await (await request.get(`/api/v1/parcels/${parcel.id}`)).json()).parcel;
  expect(archived.archived_at).not.toBeNull();
  await viewerAction(page, 'Unarchive');
  await expect(page.getByTestId('archived-badge')).toHaveCount(0);
});

test('upload sets tags and a slug', async ({ page, request }) => {
  const handle = await myHandle(request);
  const slug = uniqTag('upload');
  const tag = uniqTag('up');
  await page.goto('/new');
  await page.getByTestId('file-input').setInputFiles({
    name: 'index.html',
    mimeType: 'text/html',
    buffer: htmlPage('Uploaded with tags'),
  });
  const tags = page.getByRole('combobox', { name: 'Tags' });
  await tags.fill(tag);
  await tags.press('Enter');
  await page.getByRole('textbox', { name: 'Slug' }).fill(slug);
  await expect(page.getByTestId('slug-preview')).toHaveText(new RegExp(`/@${handle}/${slug}$`));

  await page.getByRole('button', { name: 'Publish parcel' }).click();
  await expect(page).toHaveURL(new RegExp(`/@${handle}/${slug}$`));
  await expect(
    parcelFrame(page).getByRole('heading', { name: 'Uploaded with tags' }),
  ).toBeVisible();
  const detail = await (
    await request.get(`/api/v1/parcels/by-slug?handle=${handle}&slug=${slug}`)
  ).json();
  ids.push(detail.parcel.id);
  expect(detail.parcel.tags).toEqual([tag]);
  // Published from the web: the version records it (for the owner).
  expect(detail.versions[0].source).toEqual({ client: 'web' });
  // The owner sees where it came from.
  await page.getByRole('button', { name: 'Show details' }).click();
  const source = page.getByRole('region', { name: 'Source' });
  await expect(source).toContainText('web');
});

test('the Tags page renames, merges and deletes', async ({ page, request }) => {
  const base = uniqTag('tagpage');
  const a = `${base}-alpha`;
  const ab = `${base}-alphax`; // a likely duplicate of `a` (a prefix match)
  const solo = `${base}-solo`;
  await tagged(request, uniq('e2e-t1'), { tags: [a, solo] });
  await tagged(request, uniq('e2e-t2'), { tags: [a] });
  await tagged(request, uniq('e2e-t3'), { tags: [ab] });

  await page.goto('/tags');
  await expect(page.getByRole('heading', { name: 'Tags', level: 1 })).toBeVisible();
  const row = (tag: string) => page.locator(`[data-testid="tag-row"][data-tag="${tag}"]`);
  await expect(row(a)).toContainText('2');
  await expect(row(ab)).toContainText('1');

  // The duplicate hint offers the merge.
  const hints = page.getByRole('region', { name: 'Possible duplicates' });
  await expect(hints).toContainText(ab);
  await hints.getByRole('button', { name: `Merge into ${a}…` }).click();
  await expect(row(ab)).toContainText(`Merge ${ab} (1 parcel) into ${a} (2 parcels)?`);
  await row(ab).getByRole('button', { name: 'Merge', exact: true }).click();
  await expect(row(ab)).toHaveCount(0);
  await expect(row(a)).toContainText('3');

  // A plain rename (into a new name).
  const renamed = `${base}-renamed`;
  await page.getByRole('button', { name: `Rename ${solo}` }).click();
  // The old name is selected: typing replaces it.
  const renameInput = page.getByRole('textbox', { name: `New name for ${solo}` });
  await expect(renameInput).toBeFocused();
  await page.keyboard.type(renamed);
  await expect(renameInput).toHaveValue(renamed);
  await expect(row(solo)).toContainText('Renames it on 1 parcel.');
  await row(solo).getByRole('button', { name: 'Rename', exact: true }).click();
  await expect(row(renamed)).toBeVisible();
  await expect(row(solo)).toHaveCount(0);

  // Renaming into an existing tag asks before merging.
  await page.getByRole('button', { name: `Rename ${renamed}` }).click();
  await page.getByRole('textbox', { name: `New name for ${renamed}` }).fill(a);
  await expect(row(renamed)).toContainText(`${a} already exists: renaming merges the two.`);
  await row(renamed).getByRole('button', { name: 'Merge…' }).click();
  await expect(row(renamed)).toContainText('This can’t be undone.');
  await row(renamed).getByRole('button', { name: 'Merge', exact: true }).click();
  await expect(row(renamed)).toHaveCount(0);
  // Merged without duplicates: three parcels still carry it.
  await expect(row(a)).toContainText('3');

  // Delete, after an in-page confirm.
  await page.getByRole('button', { name: `Delete ${a}` }).click();
  await expect(row(a)).toContainText(`Remove ${a} from 3 parcels?`);
  await row(a).getByRole('button', { name: 'Delete tag' }).click();
  await expect(row(a)).toHaveCount(0);
  const list = await (await request.get(`/api/v1/tags?prefix=${base}`)).json();
  expect(list.tags).toEqual([]);
});
