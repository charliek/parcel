import { expect, type Page, test } from '@playwright/test';
import {
  createParcelViaApi,
  deleteParcelViaApi,
  htmlPage,
  patchJson,
  patchParcelViaApi,
  publishViaApi,
  uniq,
  uniqTag,
  uniqWord,
} from './helpers';

const ids: string[] = [];

test.afterEach(async ({ request }) => {
  for (const id of ids.splice(0)) await deleteParcelViaApi(request, id);
});

function searchBox(page: Page) {
  return page.getByRole('combobox', { name: 'Search parcels' });
}

/** A one-page parcel with these tags; returns its id. */
async function tagged(
  request: Parameters<typeof publishViaApi>[0],
  title: string,
  tags: string[],
  body = title,
): Promise<string> {
  const result = await publishViaApi(request, {
    metadata: { title, tags },
    files: [{ path: 'index.html', body: htmlPage(body) }],
  });
  ids.push(result.parcel.id);
  return result.parcel.id as string;
}

test('dashboard loads for the dev user', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'My parcels', level: 1 })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Account menu' })).toBeVisible();
  await expect(searchBox(page)).toBeVisible();
  await expect(page.getByRole('link', { name: 'New parcel' }).first()).toBeVisible();
  await expect(page.getByRole('button', { name: 'Mine', exact: true })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
});

test('search filters parcels server-side', async ({ page, request }) => {
  const title = uniq('e2e-search');
  const id = await createParcelViaApi(request, { title, marker: title });
  ids.push(id);
  await page.goto('/');
  const search = searchBox(page);
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
});

test('facets filter the list, with counts for the current filters', async ({ page, request }) => {
  const topic = uniqTag('topic');
  const p = uniqTag('alpha');
  const q = uniqTag('beta');
  await tagged(request, uniq('e2e-a'), [`project:${p}`, 'kind:plan', topic]);
  await tagged(request, uniq('e2e-b'), [`project:${p}`, 'kind:report', topic]);
  await tagged(request, uniq('e2e-c'), [`project:${q}`, 'kind:plan', topic]);

  await page.goto(`/?tag=${topic}`);
  const facets = page.getByRole('complementary', { name: 'Filters' });
  const projects = facets.getByRole('region', { name: 'Projects' });
  const kinds = facets.getByRole('region', { name: 'Kind' });
  await expect(page.getByTestId('parcel-card')).toHaveCount(3);
  await expect(projects.locator(`[data-tag="project:${p}"]`)).toContainText('2');
  await expect(projects.locator(`[data-tag="project:${q}"]`)).toContainText('1');
  await expect(kinds.locator('[data-tag="kind:plan"]')).toContainText('2');

  // Pick a project: the list narrows and the counts follow.
  await projects.locator(`[data-tag="project:${p}"]`).click();
  await expect(page).toHaveURL(new RegExp(`tag=${topic}&tag=project%3A${p}$`));
  await expect(page.getByTestId('parcel-card')).toHaveCount(2);
  await expect(page.getByTestId('result-count')).toHaveText('2 parcels match');
  await expect(kinds.locator('[data-tag="kind:plan"]')).toContainText('1');
  await expect(kinds.locator('[data-tag="kind:report"]')).toContainText('1');
  await expect(projects.locator(`[data-tag="project:${p}"]`)).toHaveAttribute(
    'aria-pressed',
    'true',
  );

  // A second project replaces the first (a parcel has one).
  await projects.locator(`[data-tag="project:${q}"]`).click();
  await expect(page).toHaveURL(new RegExp(`tag=${topic}&tag=project%3A${q}$`));
  await expect(page.getByTestId('parcel-card')).toHaveCount(1);

  // Back and Forward walk the filters.
  await page.goBack();
  await expect(page.getByTestId('parcel-card')).toHaveCount(2);
  await page.goForward();
  await expect(page.getByTestId('parcel-card')).toHaveCount(1);

  // A tag chip on a card is a filter too; an active-filter chip removes one.
  await page.getByTestId('parcel-card').getByRole('button', { name: 'kind:plan' }).click();
  await expect(page).toHaveURL(/tag=kind%3Aplan$/);
  await page.getByRole('button', { name: `Remove the project:${q} filter` }).click();
  await expect(page).toHaveURL(new RegExp(`\\?tag=${topic}&tag=kind%3Aplan$`));
  await expect(page.getByTestId('parcel-card')).toHaveCount(2);
});

test('Back restores the search text a filter change left behind', async ({ page, request }) => {
  const first = uniqWord('qa');
  const second = uniqWord('qb');
  const tag = uniqTag('back');
  await tagged(request, uniq(`e2e-back ${first}`), [tag]);

  await page.goto(`/?q=${first}`);
  const search = searchBox(page);
  await expect(search).toHaveValue(first);
  // A filter pushes a history entry; typing then replaces that entry.
  await page.getByTestId('parcel-card').getByRole('button', { name: tag }).click();
  await expect(page).toHaveURL(new RegExp(`\\?tag=${tag}&q=${first}$`));
  await search.fill(second);
  await expect(page).toHaveURL(new RegExp(`\\?tag=${tag}&q=${second}$`));

  await page.goBack();
  await expect(page).toHaveURL(new RegExp(`\\?q=${first}$`));
  await expect(search).toHaveValue(first);
  // The text typed before Back doesn't settle back over the address.
  await page.waitForTimeout(600);
  await expect(page).toHaveURL(new RegExp(`\\?q=${first}$`));

  await page.goForward();
  await expect(page).toHaveURL(new RegExp(`\\?tag=${tag}&q=${second}$`));
  await expect(search).toHaveValue(second);
});

test('the search box autocompletes tags from the keyboard', async ({ page, request }) => {
  const stem = uniqTag('kbd');
  await tagged(request, uniq('e2e-kbd-1'), [`${stem}-one`, `${stem}-two`]);
  await tagged(request, uniq('e2e-kbd-2'), [`${stem}-one`]);

  await page.goto('/');
  const search = searchBox(page);
  await search.click();
  await search.pressSequentially(stem);
  const list = page.getByRole('listbox', { name: 'Tag suggestions' });
  await expect(list).toBeVisible();
  const options = list.getByRole('option');
  await expect(options).toHaveCount(2);
  // The most used first, and it starts active.
  await expect(options.nth(0)).toContainText(`${stem}-one`);
  await expect(options.nth(0)).toHaveAttribute('aria-selected', 'true');
  await expect(search).toHaveAttribute('aria-expanded', 'true');

  // The first ↓ lands on the highlighted option (↓ + Enter picks it, as Enter does).
  await search.press('ArrowDown');
  await expect(options.nth(0)).toHaveAttribute('aria-selected', 'true');
  await search.press('ArrowDown');
  await expect(options.nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(search).toHaveAttribute(
    'aria-activedescendant',
    (await options.nth(1).getAttribute('id')) ?? '',
  );
  await search.press('ArrowDown'); // wraps
  await expect(options.nth(0)).toHaveAttribute('aria-selected', 'true');
  await search.press('ArrowUp');
  await expect(options.nth(1)).toHaveAttribute('aria-selected', 'true');

  // Enter adds it as a filter, clears the word and keeps focus in the box.
  await search.press('Enter');
  await expect(page).toHaveURL(new RegExp(`\\?tag=${stem}-two$`));
  await expect(search).toHaveValue('');
  await expect(search).toBeFocused();
  await expect(page.getByTestId('parcel-card')).toHaveCount(1);

  // Text before the word stays as the search.
  await search.pressSequentially(`e2e kbd ${stem}-o`);
  await expect(options).toHaveCount(1);
  await search.press('Escape');
  await expect(list).toHaveCount(0);
  await expect(search).toBeFocused();
  await search.press('ArrowDown');
  await expect(list).toBeVisible();
  await search.press('Enter');
  await expect(search).toHaveValue('e2e kbd ');
  await expect(page).toHaveURL(new RegExp(`tag=${stem}-two&tag=${stem}-one&q=e2e\\+kbd$`));

  // Backspace in an empty box removes the last filter.
  await search.fill('');
  await search.press('Backspace');
  await expect(page).toHaveURL(new RegExp(`\\?tag=${stem}-two$`));
});

test('a full-text search finds page text and shows where', async ({ page, request }) => {
  const word = uniqWord('zq');
  const id = await tagged(
    request,
    uniq('e2e-fulltext'),
    [],
    `Nothing in the title. The body mentions ${word} somewhere in the middle of a sentence.`,
  );
  // Page text is indexed just after the publish commits.
  await expect
    .poll(async () => {
      const list = await (await request.get(`/api/v1/parcels?q=${word}`)).json();
      return list.parcels.map((p: { id: string }) => p.id);
    })
    .toEqual([id]);

  await page.goto('/');
  await searchBox(page).fill(word);
  const card = page.getByTestId('parcel-card');
  await expect(card).toHaveCount(1);
  const snippet = card.getByTestId('snippet');
  await expect(snippet).toContainText('The body mentions');
  await expect(snippet.locator('mark').first()).toHaveText(word);
});

test('a search nothing matches exactly falls back to any word, and says so', async ({
  page,
  request,
}) => {
  const [a, b] = [uniqWord('ra'), uniqWord('rb')];
  const topic = uniqTag('relaxed');
  // Titles are searchable as soon as the publish commits.
  await tagged(request, uniq(`e2e ${a}`), [topic, 'kind:plan']);
  await tagged(request, uniq(`e2e ${b}`), [topic, 'kind:notes']);

  // The facet requests carry the list's mode.
  const facets = page.waitForRequest(
    (r) =>
      r.url().includes('/api/v1/tags?') && new URL(r.url()).searchParams.get('mode') === 'relaxed',
  );
  await page.goto('/');
  await searchBox(page).fill(`${a} ${b}`);
  const notice = page.getByTestId('relaxed-notice');
  await expect(notice).toHaveText('No exact matches; showing parcels that match any of the words.');
  await expect(page.getByTestId('parcel-card')).toHaveCount(2);
  await facets;
  const sidebar = page.getByRole('complementary', { name: 'Filters' });
  await expect(sidebar.locator(`[data-tag="${topic}"]`)).toContainText('2');

  // Another query clears it: one word matches exactly.
  await searchBox(page).fill(a);
  await expect(page.getByTestId('parcel-card')).toHaveCount(1);
  await expect(notice).toHaveCount(0);
});

test('scope All lists your parcels and the ones shared with you', async ({ page, request }) => {
  const title = uniq('e2e-scope');
  const id = await createParcelViaApi(request, { title, marker: title });
  ids.push(id);
  const sharedTitle = uniq('e2e-shared-by-ada');
  const summary = (await (await request.get(`/api/v1/parcels/${id}`)).json()).parcel;
  // The dev user is alone on the stack: add someone else's parcel to the answer.
  await patchJson(page, /\/api\/v1\/parcels\?(.*&)?scope=all/, (body) => ({
    ...body,
    parcels: [
      ...body.parcels,
      {
        ...summary,
        id: 'zzzzzzzzzzzz',
        title: sharedTitle,
        shared: { owner_name: 'Ada Owner', role: 'commenter', shared_at: summary.created_at },
      },
    ],
    total: body.total + 1,
  }));

  await page.goto('/');
  await page.getByRole('button', { name: 'All', exact: true }).click();
  await expect(page).toHaveURL(/\?scope=all$/);
  await expect(page.getByRole('heading', { name: 'All parcels', level: 1 })).toBeVisible();
  await expect(page.getByTestId('parcel-card').filter({ hasText: title })).toBeVisible();
  await expect(page.getByTestId('parcel-card').filter({ hasText: sharedTitle })).toContainText(
    'Ada Owner · Can comment',
  );
  await page.getByRole('button', { name: 'Shared', exact: true }).click();
  await expect(page).toHaveURL(/\?scope=shared$/);
  await expect(page.getByRole('heading', { name: 'Shared with me', level: 1 })).toBeVisible();
});

test('archived parcels are hidden unless asked for', async ({ page, request }) => {
  const topic = uniqTag('arch');
  const title = uniq('e2e-archived');
  const id = await tagged(request, title, [topic]);
  await patchParcelViaApi(request, id, { archived: true });

  await page.goto(`/?tag=${topic}`);
  await expect(page.getByRole('heading', { name: 'No matching parcels' })).toBeVisible();
  await page.getByRole('checkbox', { name: 'Show archived' }).check();
  await expect(page).toHaveURL(new RegExp(`\\?tag=${topic}&archived=include$`));
  const card = page.getByTestId('parcel-card').filter({ hasText: title });
  await expect(card).toBeVisible();
  await expect(card).toContainText('Archived');
});

test('cards wrap long titles beside their badges and keep the footer whole', async ({
  page,
  request,
}) => {
  const tag = uniqTag('cards');
  const title = `${uniq('e2e-card')} build speed over time for search`;
  const id = await tagged(request, title, [tag, 'kind:report', 'project:search-infra']);
  // A comment adds its badge to the footer, and archiving a second badge by the title.
  const made = await request.post(`/api/v1/parcels/${id}/comments`, {
    data: { body: 'A note', anchor: { path: 'index.html', text: 'x' } },
  });
  expect(made.status()).toBe(201);
  await patchParcelViaApi(request, id, { archived: true });

  await page.setViewportSize({ width: 1400, height: 900 });
  await page.goto(`/?tag=${tag}&archived=include`);
  const card = page.getByTestId('parcel-card');
  await expect(card).toHaveCount(1);
  await expect(card.getByTestId('open-comments')).toBeVisible();

  // The title takes two lines (no more) instead of being cut after a word.
  const lines = await card.getByTestId('card-title').evaluate((el) => {
    const lineHeight = Number.parseFloat(getComputedStyle(el).lineHeight);
    return Math.round(el.getBoundingClientRect().height / lineHeight);
  });
  expect(lines).toBe(2);
  // The footer is one line: sizes and counts don't break inside.
  const footer = await card.getByTestId('card-footer').boundingBox();
  expect(footer?.height ?? 0).toBeLessThan(28);
});

test('at phone width the facets fold behind a Filters button', async ({ page, request }) => {
  const topic = uniqTag('phone');
  await tagged(request, uniq('e2e-phone'), [topic, 'kind:notes']);
  await page.setViewportSize({ width: 375, height: 740 });
  await page.goto(`/?tag=${topic}`);

  const filters = page.getByRole('button', { name: /^Filters/ });
  const sidebar = page.getByRole('complementary', { name: 'Filters' });
  await expect(filters).toBeVisible();
  await expect(filters).toHaveAttribute('aria-expanded', 'false');
  await expect(sidebar).toBeHidden();
  await filters.click();
  await expect(filters).toHaveAttribute('aria-expanded', 'true');
  await expect(sidebar.getByRole('region', { name: 'Kind' })).toBeVisible();
  // It opens under the toolbar and its filter chips, not above them.
  const chip = page.getByRole('button', { name: `Remove the ${topic} filter` });
  const chipBox = await chip.boundingBox();
  const sidebarBox = await sidebar.boundingBox();
  expect(sidebarBox?.y ?? 0).toBeGreaterThan(chipBox?.y ?? Number.POSITIVE_INFINITY);
  await sidebar.locator('[data-tag="kind:notes"]').click();
  await expect(page).toHaveURL(/tag=kind%3Anotes$/);
  // A pick closes it: the chips and the results are what's on screen, and
  // focus is back on the button.
  await expect(sidebar).toBeHidden();
  await expect(filters).toHaveAttribute('aria-expanded', 'false');
  await expect(filters).toBeFocused();
  await expect(page.getByRole('button', { name: 'Remove the kind:notes filter' })).toBeInViewport();
  await expect(page.getByTestId('parcel-card').first()).toBeInViewport();
  // Nothing spills sideways.
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow).toBeLessThanOrEqual(0);
});

test('account menu links to tokens, tags and CLI setup', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Account menu' }).click();
  await page.getByRole('menuitem', { name: 'Tags' }).click();
  await expect(page.getByRole('heading', { name: 'Tags', level: 1 })).toBeVisible();
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

  // A clean URL nobody has.
  await page.goto('/@nobody-here/no-such-parcel');
  await expect(page.getByRole('heading', { name: 'Parcel not found' })).toBeVisible();
});
