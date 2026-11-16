// User scenarios of the admin console, run in Chromium against the real
// lc-server binary (fresh database per run, see e2e/server.mjs). They are
// written from the mockup (docs/mockups/admin-panel) and the API contract
// (docs/rust/admin-console-api.md), not from the console code.
//
// Every page load also fails the test on a console error, a 5xx or 429
// response, or a Content-Security-Policy violation.

import { expect, test, type Page } from '@playwright/test';
import { E2E_SECRET } from '../playwright.config';

const problems: string[] = [];

function watch(page: Page) {
  page.on('console', (message) => {
    // Chromium logs every non-2xx response as "Failed to load resource"; the
    // expected ones (401 of the session probe before login) are not errors,
    // and the unexpected ones are caught by the response listener below.
    if (message.type() === 'error' && !message.text().startsWith('Failed to load resource')) {
      problems.push(`console: ${message.text()}`);
    }
  });
  page.on('pageerror', (error) => problems.push(`page error: ${error.message}`));
  page.on('response', (response) => {
    if (response.status() >= 500 || response.status() === 429) {
      problems.push(`HTTP ${response.status()} ${response.url()}`);
    }
  });
}

test.beforeEach(async ({ page }) => {
  problems.length = 0;
  watch(page);
  await page.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', (event) => {
      console.error(`CSP violation: ${event.violatedDirective} ${event.blockedURI}`);
    });
  });
});

test.afterEach(() => {
  expect(problems, problems.join('\n')).toEqual([]);
});

/** Clicks a save button and waits for the server to accept the mutation. */
async function saveAndWait(page: Page, button: string | RegExp, method: string, path: RegExp) {
  const saved = page.waitForResponse(
    (response) =>
      response.request().method() === method && path.test(new URL(response.url()).pathname),
  );
  await page.getByRole('button', { name: button }).first().click();
  expect((await saved).status()).toBe(200);
}

async function login(page: Page) {
  await page.goto('/admin/');
  await page.getByLabel('Secret d’administration').fill(E2E_SECRET);
  await page.getByRole('button', { name: 'Se connecter avec le secret' }).click();
  await expect(page.getByRole('heading', { name: 'Tableau de bord' })).toBeVisible();
}

async function createGuild(page: Page, name: string): Promise<string> {
  await page.goto('/admin/#/guilds');
  await page.getByRole('button', { name: /Créer une guilde/ }).first().click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Nom de la guilde').fill(name);
  await dialog.getByRole('button', { name: /Créer/ }).last().click();
  await expect(page.getByRole('dialog', { name: 'Guilde créée' })).toBeVisible();
  // Closing the dialog opens the new guild.
  await page.getByRole('dialog').getByRole('button', { name: 'Terminé' }).click();
  await page.waitForURL(/#\/guilds\/[^/]+/);
  await expect(page.getByRole('heading', { name, exact: true })).toBeVisible();
  const id = new URL(page.url()).hash.match(/^#\/guilds\/([^/?]+)/)?.[1];
  expect(id, `guild page ${page.url()}`).toBeTruthy();
  await page.goto('/admin/#/guilds');
  await expect(page.getByRole('link', { name, exact: true })).toBeVisible();
  return id as string;
}

test('login: a wrong secret is refused, the right one opens a session that survives a reload, logout closes it', async ({
  page,
}) => {
  await page.goto('/admin/');
  await page.getByLabel('Secret d’administration').fill('not-the-secret');
  await page.getByRole('button', { name: 'Se connecter avec le secret' }).click();
  await expect(page.getByRole('alert').or(page.locator('.notice, .error'))).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Tableau de bord' })).toHaveCount(0);

  await page.getByLabel('Secret d’administration').fill(E2E_SECRET);
  await page.getByRole('button', { name: 'Se connecter avec le secret' }).click();
  await expect(page.getByRole('heading', { name: 'Tableau de bord' })).toBeVisible();

  // The session cookie is HttpOnly: page scripts cannot read it.
  expect(await page.evaluate(() => document.cookie)).not.toContain('lc_admin_session');
  const cookies = await page.context().cookies();
  const session = cookies.find((cookie) => cookie.name === 'lc_admin_session');
  expect(session?.httpOnly).toBe(true);
  expect(session?.sameSite).toBe('Strict');
  expect(await page.evaluate(() => JSON.stringify(localStorage))).toBe('{}');

  await page.reload();
  await expect(page.getByRole('heading', { name: 'Tableau de bord' })).toBeVisible();

  await page.getByRole('button', { name: 'Se déconnecter' }).click();
  await expect(page.getByLabel('Secret d’administration')).toBeVisible();
  await page.reload();
  await expect(page.getByLabel('Secret d’administration')).toBeVisible();
});

test('API calls without a session are refused', async ({ request }) => {
  const response = await request.get('/admin/api/overview');
  expect(response.status()).toBe(401);
});

test('every screen opens without error after login', async ({ page }) => {
  await login(page);
  for (const [hash, title] of [
    ['guilds', 'Guildes'],
    ['devices', 'Overlays'],
    ['codes', 'Codes et invitations'],
    ['ingest', 'Clients d’ingestion'],
    ['roles', 'Rôles et droits'],
    ['media', 'Médias et stockage'],
    ['activity', 'Journal'],
    ['settings', 'Paramètres du serveur'],
  ]) {
    await page.goto(`/admin/#/${hash}`);
    await expect(page.getByRole('heading', { name: title, exact: true }).first()).toBeVisible();
    await expect(page.getByText('Indisponible sur ce serveur')).toHaveCount(0);
  }
});

test('a standalone guild is created, renamed, given an invitation, and appears in the journal', async ({
  page,
}) => {
  await login(page);
  const id = await createGuild(page, 'Guilde E2E');

  await page.goto(`/admin/#/guilds/${id}/settings`);
  await page.getByLabel('Nom de la guilde').fill('Guilde E2E renommée');
  await saveAndWait(page, /Enregistrer/, 'PATCH', /\/settings$/);
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Guilde E2E renommée' })).toBeVisible();

  await page.goto(`/admin/#/guilds/${id}/access`);
  await page.getByRole('button', { name: /Créer une invitation/ }).first().click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: /Créer/ }).last().click();
  await expect(page.getByRole('dialog', { name: 'Invitation créée' })).toBeVisible();
  const code = (await page.getByRole('dialog').locator('.secret span').first().textContent())?.trim();
  expect(code).toMatch(/^[A-Z0-9]{8}$/);
  await page.getByRole('dialog').getByRole('button', { name: 'Terminé' }).click();
  // The new invitation is listed on the tab.
  await expect(page.getByRole('button', { name: `Copier le code ${code}` })).toBeVisible();

  await page.goto('/admin/#/activity');
  await expect(page.getByText(/guild\.create|Création|créée/).first()).toBeVisible();
  await expect(page.getByText(/guild\.settings|Réglages/).first()).toBeVisible();
});

test('deleting a guild requires typing its identifier', async ({ page }) => {
  await login(page);
  const id = await createGuild(page, 'Guilde à supprimer');

  await page.goto(`/admin/#/guilds/${id}/danger`);
  await page.getByRole('button', { name: 'Supprimer la guilde' }).click();
  const dialog = page.getByRole('dialog');
  const confirm = dialog.getByRole('button', { name: /Supprimer/ }).last();
  await expect(confirm).toBeDisabled();
  await dialog.getByRole('textbox').fill('wrong-id');
  await expect(confirm).toBeDisabled();
  await dialog.getByRole('textbox').fill(id);
  await expect(confirm).toBeEnabled();
  await confirm.click();

  await page.goto('/admin/#/guilds');
  await expect(page.getByRole('link', { name: 'Guilde à supprimer' })).toHaveCount(0);
});

test('a right changed on the Rôles et droits page is saved on the server and can be reset', async ({
  page,
  request,
}) => {
  await login(page);
  await page.goto('/admin/#/roles');
  await page.getByRole('button', { name: 'Overlay', exact: true }).click();

  const row = page.getByRole('row', { name: /Lancer une lecture/ });
  const guestCell = row.getByRole('checkbox').first();
  const before = await guestCell.isChecked();
  await guestCell.click();
  await saveAndWait(page, 'Enregistrer', 'PUT', /^\/admin\/api\/rights$/);

  await page.reload();
  await page.getByRole('button', { name: 'Overlay', exact: true }).click();
  await expect(
    page.getByRole('row', { name: /Lancer une lecture/ }).getByRole('checkbox').first(),
  ).toBeChecked({ checked: !before });

  // The server really stores it (same cookie jar as the page).
  const rights = await page.request.get('/admin/api/rights');
  expect(rights.status()).toBe(200);
  const body = await rights.json();
  const overlay = body.surfaces.find((surface: { id: string }) => surface.id === 'overlay');
  const play = overlay.actions.find((action: { id: string }) => action.id === 'triggerPlayback');
  expect(play.cells.guest.allowed).toBe(!before);
  void request;

  await page.getByRole('button', { name: 'Valeurs par défaut' }).click();
  await saveAndWait(page, 'Enregistrer', 'PUT', /^\/admin\/api\/rights$/);
  await page.reload();
  await page.getByRole('button', { name: 'Overlay', exact: true }).click();
  await expect(
    page.getByRole('row', { name: /Lancer une lecture/ }).getByRole('checkbox').first(),
  ).toBeChecked({ checked: before });
});

test('a server setting changed in Paramètres du serveur takes effect without restart', async ({
  page,
}) => {
  await login(page);
  await page.goto('/admin/#/settings');
  const field = page.getByLabel('Durée par défaut (s)');
  await field.fill('9');
  await saveAndWait(page, /Enregistrer/, 'PATCH', /^\/admin\/api\/runtime-settings$/);
  await page.reload();
  await expect(page.getByLabel('Durée par défaut (s)')).toHaveValue('9');

  // A guild without its own duration now reports the new server default.
  const id = await createGuild(page, 'Guilde durée');
  const detail = await page.request.get(`/admin/api/guilds/${id}`);
  expect((await detail.json()).settings.defaultMediaTime).toBe(9);
});

test('the live stream pushes an update without reloading the page', async ({ page }) => {
  await login(page);
  await page.goto('/admin/#/guilds');
  await expect(page.getByRole('heading', { name: 'Guildes', exact: true }).first()).toBeVisible();

  // Another admin creates a guild through the API.
  const created = await page.request.post('/admin/api/guilds', {
    data: { name: 'Guilde temps réel' },
    headers: { Origin: new URL(page.url()).origin },
  });
  expect(created.status()).toBe(200);

  await expect(page.getByRole('link', { name: 'Guilde temps réel' })).toBeVisible({
    timeout: 4_000,
  });
});
