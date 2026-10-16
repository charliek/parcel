import { expect, test } from '@playwright/test';
import { uniq } from './helpers';

test('create a token, see the secret once, then revoke it', async ({ page }) => {
  const name = uniq('e2e-token');
  await page.goto('/tokens');
  await expect(page.getByRole('heading', { name: 'Tokens', level: 1 })).toBeVisible();

  await page.getByRole('button', { name: 'Create token' }).first().click();
  const create = page.getByRole('dialog', { name: 'Create token' });
  await create.getByLabel('Name').fill(name);
  await create.getByLabel('Expires').selectOption('30');
  await create.getByRole('button', { name: 'Create token' }).click();

  // The one-time reveal: a pcl_ secret, dismissible only by the explicit button.
  const reveal = page.getByRole('dialog', { name: 'Token created' });
  await expect(reveal.getByTestId('token-secret')).toHaveValue(/^pcl_/);
  await page.keyboard.press('Escape');
  await expect(reveal).toBeVisible();
  await reveal.getByRole('button', { name: "I've saved my token" }).click();
  await expect(reveal).toHaveCount(0);

  const row = page.getByRole('row').filter({ hasText: name });
  await expect(row).toBeVisible();
  await expect(row).toContainText('Web');

  await row.getByRole('button', { name: `Revoke ${name}` }).click();
  const confirm = page.getByRole('alertdialog');
  await expect(confirm).toContainText(name);
  await confirm.getByRole('button', { name: 'Revoke token' }).click();
  await expect(page.getByRole('row').filter({ hasText: name })).toHaveCount(0);
});
