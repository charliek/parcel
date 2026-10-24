import { expect, type Locator, type Page, test } from '@playwright/test';
import { deleteParcelViaApi, parcelFrame, publishViaApi, uniq } from './helpers';

let parcelId = '';

test.afterEach(async ({ request }) => {
  if (parcelId) await deleteParcelViaApi(request, parcelId);
  parcelId = '';
});

const ONE = `<!doctype html><html><head><meta charset="utf-8"><title>Page One</title></head>
<body><h1 id="one">Page One</h1>
<p><a id="to-two" href="two.html#part">Go to page two</a></p>
<p><a id="to-notes" href="notes.md">Notes</a></p></body></html>`;

const TWO = `<!doctype html><html><head><meta charset="utf-8"><title>Page Two</title></head>
<body><h1 id="two">Page Two</h1>
<div style="height: 1500px"></div>
<h2 id="part">Part two</h2>
<p><a id="to-one" href="index.html">Back to one</a></p></body></html>`;

const NOTES = '# Notes page\n\nSome *notes* in Markdown.\n';

async function publishTwoPages(request: Parameters<typeof publishViaApi>[0]): Promise<string> {
  const result = await publishViaApi(request, {
    metadata: { title: uniq('e2e-pages') },
    files: [
      { path: 'index.html', body: ONE },
      { path: 'two.html', body: TWO },
      { path: 'notes.md', body: NOTES },
    ],
  });
  return result.parcel.id as string;
}

function pagesMenu(page: Page): Locator {
  return page.getByTestId('pages-menu');
}

/** Counts `GET …/view` requests (each one mints a grant and reloads the frame). */
function countGrants(page: Page): { count: () => number } {
  let n = 0;
  page.on('request', (r) => {
    if (/\/api\/v1\/parcels\/[a-z2-9]{12}\/view/.test(r.url())) n += 1;
  });
  return { count: () => n };
}

test('the Pages menu lists the pages and opens them', async ({ page, request }) => {
  parcelId = await publishTwoPages(request);
  await page.goto(`/p/${parcelId}`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: 'Page One' })).toBeVisible();

  await expect(pagesMenu(page)).toContainText('Page One');
  await pagesMenu(page).click();
  const items = page.getByRole('menuitemradio');
  await expect(items).toHaveCount(3);
  // The entry first, then by path; a Markdown page's title is its heading.
  await expect(items.nth(0)).toContainText('Page One');
  await expect(items.nth(1)).toContainText('Notes page');
  await expect(items.nth(2)).toContainText('Page Two');
  await page.getByRole('menuitemradio', { name: /Page Two/ }).click();

  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}\\?page=two\\.html$`));
  await expect(frame.getByRole('heading', { name: 'Page Two' })).toBeVisible();
  await expect(pagesMenu(page)).toContainText('Page Two');

  // Back returns to the entry (the menu pushed a history entry).
  await page.goBack();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}$`));
  await expect(frame.getByRole('heading', { name: 'Page One' })).toBeVisible();
  await page.goForward();
  await expect(frame.getByRole('heading', { name: 'Page Two' })).toBeVisible();
});

test('a link inside the page updates ?page= without reloading the frame', async ({
  page,
  request,
}) => {
  parcelId = await publishTwoPages(request);
  await page.goto(`/p/${parcelId}`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: 'Page One' })).toBeVisible();
  // Mark the iframe element: a remount would replace it.
  const iframe = page.locator('iframe[data-testid="parcel-frame"]');
  await iframe.evaluate((el) => {
    el.dataset.mark = 'original';
  });
  const grants = countGrants(page);

  await frame.locator('#to-two').click();
  await expect(frame.getByRole('heading', { name: 'Page Two' })).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}\\?page=two\\.html#part$`));
  await expect(pagesMenu(page)).toContainText('Page Two');
  await expect(iframe).toHaveAttribute('data-mark', 'original');
  expect(grants.count()).toBe(0);

  // The link back to the entry drops ?page=, still without a new grant.
  await frame.locator('#to-one').click();
  await expect(frame.getByRole('heading', { name: 'Page One' })).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}$`));
  await expect(iframe).toHaveAttribute('data-mark', 'original');
  expect(grants.count()).toBe(0);

  // The frame's own history: Back returns to page two, and the address follows.
  await page.goBack();
  await expect(frame.getByRole('heading', { name: 'Page Two' })).toBeVisible();
  await expect(page).toHaveURL(/\?page=two\.html/);
});

test('reload and a copied link reopen the page and its fragment', async ({ page, request }) => {
  parcelId = await publishTwoPages(request);
  await page.goto(`/p/${parcelId}?page=two.html#part`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: 'Page Two' })).toBeVisible();
  // The fragment reached the frame: the page is scrolled to it.
  await expect(frame.locator('#part')).toBeInViewport();

  await page.reload();
  await expect(frame.getByRole('heading', { name: 'Part two' })).toBeInViewport();
  await expect(page).toHaveURL(/\?page=two\.html#part$/);

  // The details panel's link (what "Copy link" copies) keeps page and fragment.
  await page.getByRole('button', { name: 'Show details' }).click();
  const details = page.getByRole('complementary', { name: 'Parcel details' });
  await expect(details.getByTestId('canonical-link')).toHaveAttribute(
    'title',
    new RegExp(`/p/${parcelId}\\?page=two\\.html#part$`),
  );
});

test('a fragment change in the address moves the page, without reloading it', async ({
  page,
  request,
}) => {
  parcelId = await publishTwoPages(request);
  await page.goto(`/p/${parcelId}?page=two.html`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: 'Page Two' })).toBeInViewport();
  // Mark the document: a reload would replace it.
  await frame.locator('body').evaluate((body) => {
    body.dataset.mark = 'same';
  });
  const grants = countGrants(page);
  await page.getByRole('button', { name: 'Show details' }).click();
  const link = page
    .getByRole('complementary', { name: 'Parcel details' })
    .getByTestId('canonical-link');

  // An address edited by hand: a history entry that changes only the fragment.
  await page.evaluate(() => {
    window.location.hash = '#part';
  });
  await expect(frame.locator('#part')).toBeInViewport();
  await expect(page).toHaveURL(/\?page=two\.html#part$/);
  await expect(link).toHaveAttribute('title', /\?page=two\.html#part$/);

  await page.evaluate(() => {
    window.location.hash = '#two';
  });
  await expect(frame.locator('#part')).not.toBeInViewport();
  await expect(frame.locator('#two')).toBeInViewport();
  await expect(link).toHaveAttribute('title', /\?page=two\.html#two$/);

  // Back and Forward over those entries move the page too.
  await page.goBack();
  await expect(page).toHaveURL(/\?page=two\.html#part$/);
  await expect(frame.locator('#part')).toBeInViewport();
  await expect(link).toHaveAttribute('title', /\?page=two\.html#part$/);
  await page.goForward();
  await expect(page).toHaveURL(/\?page=two\.html#two$/);
  await expect(frame.locator('#part')).not.toBeInViewport();
  await expect(link).toHaveAttribute('title', /\?page=two\.html#two$/);

  // One document all along, and no new grant.
  await expect(frame.locator('body')).toHaveAttribute('data-mark', 'same');
  expect(grants.count()).toBe(0);
});

test('a page that is not in the version shows not found, with the entry a click away', async ({
  page,
  request,
}) => {
  parcelId = await publishTwoPages(request);
  await page.goto(`/p/${parcelId}?page=missing.html`);
  await expect(page.getByRole('heading', { name: 'Page not found' })).toBeVisible();
  await expect(page.getByText('missing.html')).toBeVisible();
  await page.getByRole('link', { name: 'Open the first page' }).click();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}$`));
  await expect(parcelFrame(page).getByRole('heading', { name: 'Page One' })).toBeVisible();
});

test('Files rows open pages; a Markdown page renders styled', async ({ page, request }) => {
  parcelId = await publishTwoPages(request);
  await page.goto(`/p/${parcelId}`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: 'Page One' })).toBeVisible();

  await page.getByRole('button', { name: 'Show details' }).click();
  const details = page.getByRole('complementary', { name: 'Parcel details' });
  await details.getByRole('button', { name: 'Open page notes.md' }).click();
  await expect(page).toHaveURL(/\?page=notes\.md$/);
  // Rendered, not raw: a real heading, no Markdown syntax on screen.
  await expect(frame.getByRole('heading', { name: 'Notes page', level: 1 })).toBeVisible();
  await expect(frame.locator('em')).toHaveText('notes');
  await expect(frame.getByText('# Notes page')).toHaveCount(0);
  await expect(details.getByRole('button', { name: 'Open page notes.md' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  // Download stays a separate button.
  await expect(details.getByRole('button', { name: 'Download notes.md' })).toBeVisible();
});

test('a comment on page two, opened from page one, opens that page and focuses it', async ({
  page,
  request,
}) => {
  parcelId = await publishTwoPages(request);
  const made = await request.post(`/api/v1/parcels/${parcelId}/comments`, {
    data: {
      body: 'On the second page',
      // Far down the page: only the viewer's `focus` scrolls it into view.
      anchor: { path: 'two.html', selector: 'h2#part', text: 'Part two' },
    },
  });
  expect(made.status()).toBe(201);
  const gone = await request.post(`/api/v1/parcels/${parcelId}/comments`, {
    data: { body: 'On a page that left', anchor: { path: 'gone.html', text: 'Old' } },
  });
  expect(gone.status()).toBe(201);

  await page.goto(`/p/${parcelId}`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: 'Page One' })).toBeVisible();
  await page
    .getByRole('banner')
    .getByRole('button', { name: /^Comment/ })
    .click();
  const panel = page.getByRole('complementary', { name: 'Comments' });

  // A thread on a page this version doesn't have says so and isn't a link.
  const old = panel.getByRole('article', { name: /^Comment #2 / });
  await expect(old).toContainText('on gone.html — not in this version');
  await expect(old.getByRole('button', { name: /Open the page/ })).toHaveCount(0);

  const thread = panel.getByRole('article', { name: /^Comment #1 / });
  await expect(thread).toContainText('on two.html');
  await thread.getByRole('button', { name: /Open the page and show it/ }).click();

  await expect(page).toHaveURL(/\?page=two\.html$/);
  await expect(thread).toBeFocused();
  // Page two opened at its top, and was scrolled to the thread's spot once
  // it proved itself (`ready`, then `focus`).
  await expect(frame.locator('#part')).toBeInViewport();
  // Now on its page: the thread's spot is a "show on the page" button again.
  await expect(thread.getByRole('button', { name: 'Show #1 on the page' })).toBeVisible();
  // And the page drew its pin over the heading.
  await expect
    .poll(() =>
      frame.locator('#part').evaluate((el) => {
        const r = el.getBoundingClientRect();
        const hit = document.elementFromPoint(r.left + r.width / 2 + 10, r.top + r.height / 2 - 10);
        return hit?.localName === 'parcel-overlay';
      }),
    )
    .toBe(true);
});

test('switching pages with an unsent comment asks first', async ({ page, request }) => {
  parcelId = await publishTwoPages(request);
  await page.goto(`/p/${parcelId}`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: 'Page One' })).toBeVisible();
  await page
    .getByRole('banner')
    .getByRole('button', { name: /^Comment/ })
    .click();
  const panel = page.getByRole('complementary', { name: 'Comments' });
  await panel.getByRole('textbox', { name: 'Add a comment' }).fill('Half a thought');

  await pagesMenu(page).click();
  await page.getByRole('menuitemradio', { name: /Page Two/ }).click();
  const dialog = page.getByRole('alertdialog', { name: 'Discard your comment?' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Keep writing' }).click();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}$`));
  await expect(panel.getByRole('textbox', { name: 'Add a comment' })).toHaveValue('Half a thought');

  await pagesMenu(page).click();
  await page.getByRole('menuitemradio', { name: /Page Two/ }).click();
  await dialog.getByRole('button', { name: 'Discard and open' }).click();
  await expect(page).toHaveURL(/\?page=two\.html$/);
  await expect(frame.getByRole('heading', { name: 'Page Two' })).toBeVisible();
  await expect(panel.getByRole('textbox', { name: 'Add a comment' })).toHaveValue('');
});

test('switching versions with an unsent comment asks first', async ({ page, request }) => {
  parcelId = await publishTwoPages(request);
  // v2 adds a page that v1 doesn't have.
  await publishViaApi(request, {
    parcel: parcelId,
    metadata: { mode: 'merge' },
    files: [{ path: 'three.html', body: TWO.replaceAll('Page Two', 'Page Three') }],
  });
  await page.goto(`/p/${parcelId}?page=three.html`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: 'Page Three' })).toBeVisible();
  await page
    .getByRole('banner')
    .getByRole('button', { name: /^Comment/ })
    .click();
  const composer = page
    .getByRole('complementary', { name: 'Comments' })
    .getByRole('textbox', { name: 'Add a comment' });
  await composer.fill('Half a thought');
  const dialog = page.getByRole('alertdialog', { name: 'Discard your comment?' });

  // The version menu: v1 has no three.html, so the comment would go.
  await page.getByRole('button', { name: /^Version 2, choose another version/ }).click();
  await page.getByRole('menuitemradio', { name: /^v1\b/ }).click();
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Keep writing' }).click();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}\\?page=three\\.html$`));

  // The history in the details panel asks the same.
  await page.getByRole('button', { name: 'Show details' }).click();
  const details = page.getByRole('complementary', { name: 'Parcel details' });
  await details.getByRole('link', { name: /^v1 / }).click();
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Keep writing' }).click();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}\\?page=three\\.html$`));
  await page.getByRole('button', { name: 'Show comments' }).click();
  await expect(composer).toHaveValue('Half a thought');

  await page.getByRole('button', { name: /^Version 2, choose another version/ }).click();
  await page.getByRole('menuitemradio', { name: /^v1\b/ }).click();
  await dialog.getByRole('button', { name: 'Discard and open' }).click();
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}/v/1\\?page=three\\.html$`));
  await expect(page.getByRole('heading', { name: 'Page not found' })).toBeVisible();
});

test('a new version reloads the page in view while it still exists', async ({ page, request }) => {
  parcelId = await publishTwoPages(request);
  await page.goto(`/p/${parcelId}?page=two.html`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: 'Page Two' })).toBeVisible();

  // v2 keeps two.html (changed): the viewer stays on it.
  await publishViaApi(request, {
    parcel: parcelId,
    metadata: { mode: 'merge' },
    files: [{ path: 'two.html', body: TWO.replace('>Page Two</h1>', '>Page Two, again</h1>') }],
  });
  await expect(frame.getByRole('heading', { name: 'Page Two, again' })).toBeVisible({
    timeout: 20_000,
  });
  await expect(page).toHaveURL(/\?page=two\.html$/);

  // v3 drops it: the viewer falls back to the entry.
  await publishViaApi(request, {
    parcel: parcelId,
    metadata: { mode: 'merge', remove: ['two.html'] },
    files: [],
  });
  await expect(frame.getByRole('heading', { name: 'Page One' })).toBeVisible({ timeout: 20_000 });
  await expect(page).toHaveURL(new RegExp(`/p/${parcelId}$`));
});
