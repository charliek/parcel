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
  {
    title,
    marker,
    visibility = 'private',
    html,
  }: { title: string; marker: string; visibility?: string; html?: Buffer },
): Promise<string> {
  const response = await request.post('/api/v1/parcels', {
    multipart: {
      metadata: JSON.stringify({ title, visibility }),
      file: { name: 'index.html', mimeType: 'text/html', buffer: html ?? htmlPage(marker) },
    },
  });
  expect(response.status(), await response.text()).toBe(201);
  const body = await response.json();
  return body.parcel.id as string;
}

/** Publish a new `index.html` for a parcel through the API; returns the version number. */
export async function publishVersionViaApi(
  request: APIRequestContext,
  id: string,
  html: Buffer,
): Promise<number> {
  const response = await request.post(`/api/v1/parcels/${id}/versions`, {
    multipart: {
      metadata: JSON.stringify({}),
      file: { name: 'index.html', mimeType: 'text/html', buffer: html },
    },
  });
  expect(response.status(), await response.text()).toBe(201);
  const body = await response.json();
  return body.version.version as number;
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

/** Pick who can open the parcel from the viewer's overflow menu. */
export async function chooseGeneralAccess(page: Page, label: string): Promise<void> {
  await page.getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitemradio', { name: label }).click();
}

/** Rewrite a JSON API response on its way to the page (the request still reaches the server). */
export async function patchJson(
  page: Page,
  url: string | RegExp,
  patch: (body: any) => unknown,
): Promise<void> {
  await page.route(url, async (route) => {
    const response = await route.fetch();
    const body = await response.json();
    await route.fulfill({ response, json: patch(body) });
  });
}

export interface StubProvider {
  id: string;
  label: string;
}

/**
 * Make the SPA believe the server signs people in through these providers
 * (the e2e stack itself runs in dev auth mode), optionally with public
 * links turned off. Call before the first `page.goto`: the app reads
 * `/api/v1/info` once at boot.
 */
export async function stubServerInfo(
  page: Page,
  { providers, allowPublic }: { providers?: StubProvider[]; allowPublic?: boolean },
): Promise<void> {
  await patchJson(page, '**/api/v1/info', (info) => ({
    ...info,
    ...(providers ? { auth: { mode: 'oidc', providers } } : {}),
    ...(allowPublic !== undefined ? { allow_public: allowPublic } : {}),
  }));
}

/** Make `/api/v1/me` answer as it does for a signed-out browser. */
export async function stubSignedOut(page: Page): Promise<void> {
  await page.route('**/api/v1/me', (route) =>
    route.fulfill({
      status: 401,
      json: { error: { message: 'sign in or pass a bearer token', type: 'Unauthorized' } },
    }),
  );
}

/**
 * Answer navigations to `/auth/login` with a stand-in page (the provider)
 * and collect the URLs asked for.
 */
export async function catchSignIn(page: Page): Promise<URL[]> {
  const seen: URL[] = [];
  await page.route('**/auth/login**', async (route) => {
    seen.push(new URL(route.request().url()));
    await route.fulfill({
      contentType: 'text/html',
      body: '<!doctype html><title>Provider</title><h1>At the provider</h1>',
    });
  });
  return seen;
}
