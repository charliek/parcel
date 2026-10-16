import { type APIRequestContext, expect, type FrameLocator, type Page } from '@playwright/test';

/** A unique, readable suffix so test data never collides across runs. */
export function uniq(prefix: string): string {
  const rand = Math.floor(Math.random() * 1e9).toString(36);
  return `${prefix}-${Date.now().toString(36)}-${rand}`;
}

/** A tiny HTML page whose <h1> carries `marker` (and optionally links a stylesheet). */
export function htmlPage(marker: string, opts: { stylesheet?: string } = {}): Buffer {
  const css = opts.stylesheet ? `<link rel="stylesheet" href="${opts.stylesheet}">` : '';
  return Buffer.from(
    `<!doctype html><html><head><meta charset="utf-8"><title>${marker}</title>${css}</head>` +
      `<body><h1 id="marker">${marker}</h1></body></html>`,
  );
}

export const PARCEL_URL = /\/p\/[a-z2-9]{12}$/;

/** The id from a viewer URL. */
export function parcelIdFrom(page: Page): string {
  const match = new URL(page.url()).pathname.match(/^\/p\/([a-z2-9]{12})/);
  if (!match) throw new Error(`not a viewer URL: ${page.url()}`);
  return match[1];
}

/** The sandboxed iframe on the viewer. */
export function parcelFrame(page: Page): FrameLocator {
  return page.frameLocator('iframe[data-testid="parcel-frame"]');
}

/** Create a single-file parcel through the API (dev auth: no token needed). */
export async function createParcelViaApi(
  request: APIRequestContext,
  { title, marker, visibility = 'private' }: { title: string; marker: string; visibility?: string },
): Promise<string> {
  const response = await request.post('/api/v1/parcels', {
    multipart: {
      metadata: JSON.stringify({ title, visibility }),
      file: { name: 'index.html', mimeType: 'text/html', buffer: htmlPage(marker) },
    },
  });
  expect(response.status(), await response.text()).toBe(201);
  const body = await response.json();
  return body.parcel.id as string;
}

/** Best-effort cleanup. */
export async function deleteParcelViaApi(request: APIRequestContext, id: string): Promise<void> {
  await request.delete(`/api/v1/parcels/${id}`).catch(() => undefined);
}

/** Open the viewer's overflow menu and pick an item. */
export async function viewerAction(page: Page, item: string | RegExp): Promise<void> {
  await page.getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitem', { name: item }).click();
}
