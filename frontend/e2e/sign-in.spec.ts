import { expect, test } from '@playwright/test';
import { catchSignIn, patchJson, stubServerInfo, stubSignedOut } from './helpers';

// The e2e stack runs in dev auth mode, where there is nothing to sign in
// to. These specs rewrite `/api/v1/info` (and `/api/v1/me`) so the SPA
// shows its sign-in UI, and catch the navigations to the server's
// `/auth/*` routes.

const okta = { id: 'okta', label: 'Acme Okta' };
const keycloak = { id: 'keycloak', label: 'Partner Keycloak' };
const slauth = { id: 'slauth', label: 'slauth' };

test('the sign-in page explains each failure', async ({ page }) => {
  await stubServerInfo(page, { providers: [okta, keycloak] });
  await stubSignedOut(page);
  const cases: [string, string][] = [
    ['denied', 'Sign-in was cancelled'],
    ['expired', 'Sign-in took too long'],
    ['domain', 'That account can’t sign in here'],
    ['unverified', 'Your email couldn’t be confirmed'],
    ['failed', 'Sign-in didn’t complete'],
    ['unavailable', 'The sign-in provider is unavailable'],
    ['disabled', 'Your account is disabled'],
    ['something-new', 'Sign-in didn’t complete'],
  ];
  for (const [code, title] of cases) {
    await page.goto(`/signin?error=${code}&return_to=%2Fp%2Fabcdefghijkm`);
    const alert = page.getByRole('alert');
    await expect(alert, code).toContainText(title);
    // The options are still there to try again, and a way back.
    await expect(page.getByRole('link', { name: 'Sign in with Acme Okta' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Go back' })).toHaveAttribute(
      'href',
      '/p/abcdefghijkm',
    );
  }
  await expect(page.getByRole('alert')).toContainText('Something went wrong while signing you in');
});

test('a failure while still signed in says so', async ({ page }) => {
  await stubServerInfo(page, { providers: [okta, keycloak] });
  await page.goto('/signin?error=domain&return_to=%2F');
  await expect(page.getByRole('alert')).toContainText('You’re still signed in as e2e@example.com');
  await expect(page.getByRole('link', { name: 'Back to Parcel' })).toHaveAttribute('href', '/');
});

test('several providers: one button each, the one used last first', async ({ page }) => {
  await stubServerInfo(page, { providers: [okta, keycloak, slauth] });
  await stubSignedOut(page);
  await page.addInitScript(() => {
    try {
      if (!sessionStorage.getItem('seeded')) {
        localStorage.setItem('parcel_last_provider', 'keycloak');
        sessionStorage.setItem('seeded', '1');
      }
    } catch {
      // storage blocked: the test will say so
    }
  });
  const seen = await catchSignIn(page);

  await page.goto('/signin?return_to=%2Fp%2Fabcdefghijkm');
  await expect(page.getByRole('heading', { name: 'Sign in to Parcel' })).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);
  const options = page.getByRole('list', { name: 'Sign-in options' }).getByRole('link');
  await expect(options).toHaveText([
    /Sign in with Partner Keycloak\s*Last used/,
    'Sign in with Acme Okta',
    'Sign in with slauth',
  ]);
  await expect(options.first()).toHaveAccessibleName('Sign in with Partner Keycloak (last used)');
  await expect(options.nth(1)).toHaveAttribute(
    'href',
    '/auth/login?provider=okta&return_to=%2Fp%2Fabcdefghijkm',
  );

  // Picking one goes to the server's login, and is remembered.
  await options.nth(1).click();
  await expect(page.getByRole('heading', { name: 'At the provider' })).toBeVisible();
  expect(seen.map((u) => `${u.pathname}${u.search}`)).toEqual([
    '/auth/login?provider=okta&return_to=%2Fp%2Fabcdefghijkm',
  ]);
  await page.goto('/signin');
  await expect(options.first()).toHaveAccessibleName('Sign in with Acme Okta (last used)');
});

test('“Sign in” picks a provider first when there are several', async ({ page }) => {
  await stubServerInfo(page, { providers: [okta, keycloak] });
  await stubSignedOut(page);
  await page.goto('/');
  await page.getByRole('banner').getByRole('button', { name: 'Sign in' }).click();
  await expect(page).toHaveURL(/\/signin\?return_to=%2F$/);
  await expect(page.getByRole('link', { name: 'Sign in with Acme Okta' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Sign in with Partner Keycloak' })).toBeVisible();
});

test('“Sign in” goes straight to the only provider', async ({ page }) => {
  await stubServerInfo(page, { providers: [slauth] });
  await stubSignedOut(page);
  const seen = await catchSignIn(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Sign in to Parcel' }).click();
  await expect(page.getByRole('heading', { name: 'At the provider' })).toBeVisible();
  expect(seen.map((u) => `${u.pathname}${u.search}`)).toEqual([
    '/auth/login?provider=slauth&return_to=%2F',
  ]);
});

test('an account page offers sign-in, and never loops through the provider', async ({ page }) => {
  await stubServerInfo(page, { providers: [slauth] });
  await stubSignedOut(page);
  const seen = await catchSignIn(page);

  // A first-time visitor sees the screen, with what the page is for.
  await page.goto('/device?code=BCDF-GHJK');
  await expect(page.getByRole('heading', { name: 'Sign in to connect the CLI' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Sign in with slauth' })).toHaveAttribute(
    'href',
    '/auth/login?provider=slauth&return_to=%2Fdevice%3Fcode%3DBCDF-GHJK',
  );
  expect(seen).toHaveLength(0);

  // A browser that signed in before goes straight back through the provider...
  await page.evaluate(() => localStorage.setItem('parcel_signed_in', '1'));
  await page.goto('/tokens');
  await expect(page.getByRole('heading', { name: 'At the provider' })).toBeVisible();
  expect(seen.map((u) => `${u.pathname}${u.search}`)).toEqual([
    '/auth/login?provider=slauth&return_to=%2Ftokens',
  ]);

  // ...but if that didn't stick, the next visit shows the screen instead.
  await page.goto('/tokens');
  await expect(page.getByRole('heading', { name: 'Sign in to Parcel' })).toBeVisible();
  await expect(page.getByText('Sign in to manage your API tokens.')).toBeVisible();
  expect(seen).toHaveLength(1);
});

test('sign out, and sign out everywhere', async ({ page }) => {
  await stubServerInfo(page, { providers: [okta] });
  await patchJson(page, '**/api/v1/me', (me) => ({ ...me, signed_in_with: 'Acme Okta' }));
  const logouts: string[] = [];
  await page.route('**/auth/logout**', async (route) => {
    const url = new URL(route.request().url());
    logouts.push(`${route.request().method()} ${url.pathname}${url.search}`);
    await route.fulfill({ json: { redirect: '/?signed-out=1' } });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'Account menu' }).click();
  await expect(page.getByTestId('signed-in-with')).toHaveText('Signed in with Acme Okta');
  await page.getByRole('menuitem', { name: 'Sign out' }).click();
  await expect(page).toHaveURL(/\/\?signed-out=1$/);
  expect(logouts).toEqual(['POST /auth/logout']);

  await page.goto('/tokens');
  const section = page.getByRole('region', { name: 'Web sign-in' });
  await expect(section).toContainText('Signed in as e2e@example.com with Acme Okta');
  await section.getByRole('button', { name: 'Sign out everywhere' }).click();
  const confirm = page.getByRole('alertdialog', { name: 'Sign out everywhere?' });
  await confirm.getByRole('button', { name: 'Sign out everywhere' }).click();
  await expect(page).toHaveURL(/\/\?signed-out=1$/);
  expect(logouts).toEqual(['POST /auth/logout', 'POST /auth/logout?everywhere=true']);
});

test('dev mode has no sign-in or sign-out', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Account menu' }).click();
  await expect(page.getByRole('menuitem', { name: 'Sign out' })).toHaveCount(0);
  await page.keyboard.press('Escape');

  // The sign-in page just moves on.
  await page.goto('/signin?return_to=%2Ftokens');
  await expect(page).toHaveURL(/\/tokens$/);
  await expect(page.getByRole('heading', { name: 'Tokens', level: 1 })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Web sign-in' })).toHaveCount(0);
});
