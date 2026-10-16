import { expect, test } from '@playwright/test';
import { createParcelViaApi, deleteParcelViaApi, parcelFrame, uniq } from './helpers';

const ids: string[] = [];

test.afterEach(async ({ request }) => {
  for (const id of ids.splice(0)) await deleteParcelViaApi(request, id);
});

/** Shows, in `#got`, the type of every message the viewer posts to this document. */
const RECORDER = `<p id="got"></p><script>
  const got = [];
  addEventListener('message', (e) => {
    if (e.data && e.data.source === 'parcel-viewer') {
      got.push(e.data.type);
      document.getElementById('got').textContent = got.join(',');
    }
  });
</script>`;

function page(marker: string, body: string): Buffer {
  return Buffer.from(
    `<!doctype html><html><head><meta charset="utf-8"><title>${marker}</title></head>` +
      `<body><h1 id="marker">${marker}</h1>${body}${RECORDER}</body></html>`,
  );
}

test('the viewer talks only to the parcel page it loaded', async ({
  page: viewer,
  request,
  baseURL,
}) => {
  // B stands in for any other document the frame can be navigated to: it is
  // sandboxed on the same "null" origin, but its token isn't the viewer's.
  // It also forges a `ready` of its own.
  const markerB = uniq('elsewhere');
  const idB = await createParcelViaApi(request, {
    title: uniq('e2e-bridge-b'),
    marker: markerB,
    html: page(
      markerB,
      `<script>parent.postMessage({ source: 'parcel-helper', type: 'ready', path: 'index.html',
        title: 'x', version: 1, token: 'forged' }, '*');</script>`,
    ),
  });
  ids.push(idB);
  const grantB = await (await request.get(`/api/v1/parcels/${idB}/view`)).json();

  const markerA = uniq('parcel');
  const idA = await createParcelViaApi(request, {
    title: uniq('e2e-bridge-a'),
    marker: markerA,
    html: page(
      markerA,
      `<p><a id="ext" href="${baseURL}/">Another origin</a></p>` +
        `<p><a id="to-b" href="${grantB.content_url}">Same origin, another page</a></p>`,
    ),
  });
  ids.push(idA);
  // A pin on A's index.html (which is also B's path).
  const comment = await request.post(`/api/v1/parcels/${idA}/comments`, {
    data: { body: 'pin me', anchor: { path: 'index.html', selector: 'h1#marker', text: markerA } },
  });
  expect(comment.status()).toBe(201);

  await viewer.goto(`/p/${idA}`);
  const frame = parcelFrame(viewer);
  await expect(frame.getByRole('heading', { name: markerA })).toBeVisible();

  // The real page proves itself and gets the state.
  const commentButton = viewer.getByRole('banner').getByRole('button', { name: /^Comment/ });
  await commentButton.click();
  await expect(frame.locator('#got')).toContainText('comment-mode');
  await expect(frame.locator('#got')).toContainText('pins');
  await viewer.getByRole('button', { name: 'Done' }).click();

  // A link to another origin opens a new tab; the frame stays on the parcel.
  const popup = viewer.context().waitForEvent('page');
  await frame.locator('#ext').click();
  await (await popup).close();
  await expect(frame.locator('#ext')).toHaveAttribute('target', '_blank');
  await expect(frame.locator('#ext')).toHaveAttribute('rel', 'noopener noreferrer');
  await expect(frame.getByRole('heading', { name: markerA })).toBeVisible();

  // A same-origin link navigates the frame, to a document the viewer didn't load.
  await frame.locator('#to-b').click();
  await expect(frame.getByRole('heading', { name: markerB })).toBeVisible();
  await expect(frame.locator('#got')).toHaveText(/^hello(,hello)*$/);

  // Nothing but `hello` reaches it, whatever the viewer does next; and its
  // clicks don't become comments.
  await commentButton.click();
  await commentButton.click();
  await expect(viewer.getByText('Pick a spot on the page to comment on it.')).toBeVisible();
  await frame.getByRole('heading', { name: markerB }).click();
  await viewer.waitForTimeout(1500);
  await expect(frame.locator('#got')).toHaveText(/^hello(,hello)*$/);
  const panel = viewer.getByRole('complementary', { name: 'Comments' });
  await expect(panel.getByRole('textbox', { name: 'Add a comment' })).toBeVisible();
  await expect(panel.getByRole('textbox', { name: 'Comment on this spot' })).toHaveCount(0);
});
