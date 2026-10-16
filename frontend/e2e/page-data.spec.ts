import { expect, test } from '@playwright/test';
import { createParcelViaApi, deleteParcelViaApi, parcelFrame, uniq } from './helpers';

let parcelId = '';

test.afterEach(async ({ request }) => {
  if (parcelId) await deleteParcelViaApi(request, parcelId);
  parcelId = '';
});

/** A page that uses the sandbox-safe storage stand-ins and `window.parcel.data`. */
function dataPage(marker: string): Buffer {
  return Buffer.from(`<!doctype html><html><head><meta charset="utf-8"><title>${marker}</title></head>
<body><h1 id="marker">${marker}</h1><p id="storage">pending</p><p id="data">pending</p>
<script>
  try {
    localStorage.setItem('theme', 'dark');
    sessionStorage.setItem('tab', '1');
    document.cookie = 'seen=yes';
    document.getElementById('storage').textContent =
      'storage ok: ' + localStorage.getItem('theme') + ' ' + document.cookie;
  } catch (err) {
    document.getElementById('storage').textContent = 'storage threw: ' + err;
  }
  (async () => {
    const out = document.getElementById('data');
    try {
      await window.parcel.data.set('votes', 'alice', { n: 3, pick: 'blue' });
      const value = await window.parcel.data.get('votes', 'alice');
      const missing = await window.parcel.data.get('votes', 'nobody');
      out.textContent = 'stored ' + JSON.stringify(value) + ' missing=' + String(missing) +
        ' canWrite=' + window.parcel.canWrite;
    } catch (err) {
      out.textContent = 'data error: ' + err.message;
    }
  })();
</script></body></html>`);
}

test('a page keeps data with window.parcel.data and storage does not throw', async ({
  page,
  request,
}) => {
  const marker = uniq('data-page');
  parcelId = await createParcelViaApi(request, {
    title: uniq('e2e-data'),
    marker,
    html: dataPage(marker),
  });

  await page.goto(`/p/${parcelId}`);
  const frame = parcelFrame(page);
  await expect(frame.getByRole('heading', { name: marker })).toBeVisible();
  await expect(frame.locator('#storage')).toHaveText('storage ok: dark seen=yes');
  // The owner's view can write; the value comes back from the server.
  await expect(frame.locator('#data')).toHaveText(
    'stored {"n":3,"pick":"blue"} missing=undefined canWrite=true',
  );

  const item = await request.get(`/api/v1/parcels/${parcelId}/data/votes/alice`);
  expect(item.status()).toBe(200);
  expect((await item.json()).value).toEqual({ n: 3, pick: 'blue' });

  // The details panel lists the collection.
  await page.getByRole('button', { name: 'Show details' }).click();
  const details = page.getByRole('complementary', { name: 'Parcel details' });
  const data = details.getByRole('region', { name: 'Data' });
  await expect(data.getByRole('list', { name: 'Data collections' })).toContainText('votes');
  await expect(data).toContainText('1 item');
  await expect(data).toContainText('of 10 MB used');
});
