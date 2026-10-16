import { expect, test } from '@playwright/test';

interface DeviceStart {
  device_code: string;
  user_code: string;
}

test('approve a CLI device login from the link the CLI prints', async ({ page, request }) => {
  const start = await request.post('/api/v1/cli/device', { data: { client_name: 'e2e laptop' } });
  expect(start.ok()).toBe(true);
  const { device_code, user_code } = (await start.json()) as DeviceStart;

  // Lowercase and without the dash: the page normalizes it.
  await page.goto(`/device?code=${user_code.toLowerCase().replace('-', '')}`);
  await expect(page.getByRole('heading', { name: 'Approve CLI login?' })).toBeVisible();
  await expect(page.getByText('e2e laptop')).toBeVisible();
  await expect(page.getByTestId('device-code')).toHaveText(user_code);
  await page.getByRole('button', { name: 'Approve' }).click();
  await expect(page.getByRole('heading', { name: 'CLI connected' })).toBeVisible();
  await expect(page.getByText('You can return to your terminal')).toBeVisible();

  const poll = await request.post('/api/v1/cli/device/token', { data: { device_code } });
  const body = await poll.json();
  expect(body.status).toBe('approved');
  expect(body.token).toMatch(/^pcl_/);

  // Clean up: the CLI token revokes itself.
  const revoke = await request.delete('/api/v1/tokens/current', {
    headers: { Authorization: `Bearer ${body.token}` },
  });
  expect(revoke.status()).toBe(204);
});

test('deny a CLI device login typed in by hand', async ({ page, request }) => {
  const start = await request.post('/api/v1/cli/device', { data: { client_name: 'e2e denied' } });
  const { device_code, user_code } = (await start.json()) as DeviceStart;

  await page.goto('/device');
  await expect(page.getByRole('heading', { name: 'Connect the Parcel CLI' })).toBeVisible();
  await page.getByRole('textbox').fill(user_code.toLowerCase().replace('-', ' '));
  await page.getByRole('button', { name: 'Continue' }).click();
  await expect(page).toHaveURL(new RegExp(`code=${user_code}`));
  await page.getByRole('button', { name: 'Deny' }).click();
  await expect(page.getByRole('heading', { name: 'Login request denied' })).toBeVisible();

  const poll = await request.post('/api/v1/cli/device/token', { data: { device_code } });
  expect((await poll.json()).status).toBe('denied');
});

test('an unknown code explains how to get a new one', async ({ page }) => {
  await page.goto('/device?code=ZZZZ-ZZZZ');
  await expect(page.getByRole('heading', { name: 'Code expired or not found' })).toBeVisible();
  await page.getByRole('button', { name: 'Enter a different code' }).click();
  await expect(page.getByRole('heading', { name: 'Connect the Parcel CLI' })).toBeVisible();
});
