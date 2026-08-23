import { test, expect, type Page } from '@playwright/test';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

/**
 * Fixtures are produced by `cargo run --bin gen-fixtures --features synthetic`,
 * which CI runs before this suite. They are synthetic by construction — real saves
 * are gitignored because they carry SteamIDs and player names.
 */
const FIXTURES = path.resolve(fileURLToPath(new URL('.', import.meta.url)), 'fixtures');
const LEVEL = path.join(FIXTURES, 'Level.sav');
const PLAYER = path.join(FIXTURES, 'Players', '00000000000000000000000000000001.sav');

/** Loading is async through a worker; the filebar appearing is the ready signal. */
async function load(page: Page, files: string[]) {
  await page.getByTestId('file-input').setInputFiles(files);
  await expect(page.getByTestId('filebar')).toBeVisible();
}

test.beforeEach(async ({ page }) => {
  // Any uncaught page error is a failure, whether or not an assertion notices it.
  // Without this a screen could throw on mount and the test would still pass.
  page.on('pageerror', (err) => {
    throw new Error(`uncaught page error: ${err.message}`);
  });
  await page.goto('/');
});

test('loads, boots wasm, and shows the dropzone', async ({ page }) => {
  await expect(page.getByTestId('dropzone')).toBeVisible();
  await expect(page.getByTestId('filebar')).toHaveCount(0);
});

test('opening a level save populates the inspector and enables tabs', async ({ page }) => {
  await load(page, [LEVEL]);

  await expect(page.getByTestId('filename')).toHaveText('Level.sav');
  // Proves the wasm actually parsed the file, not merely that the UI re-rendered.
  await expect(page.getByTestId('inspector-rows')).toContainText('5.1.1');
  await expect(page.getByTestId('inspector-rows')).toContainText('PalWorldSaveGame');

  await expect(page.getByTestId('tab-players')).toBeEnabled();
  await expect(page.getByTestId('tab-guilds')).toBeEnabled();
});

test('players tab lists the character decoded from the save', async ({ page }) => {
  await load(page, [LEVEL]);
  await page.getByTestId('tab-players').click();
  // The heading specifically — the name also appears in the player list button, and
  // an ambiguous locator is a test that breaks for the wrong reasons later.
  await expect(page.getByRole('heading', { name: 'Tester' })).toBeVisible();
  await expect(page.getByText('Level 34')).toBeVisible();
});

test('guilds tab reaches the guild rename control', async ({ page }) => {
  await load(page, [LEVEL]);
  await page.getByTestId('tab-guilds').click();
  await page.getByRole('button', { name: /Original Name/ }).click();
  await expect(page.getByLabel('Guild name')).toHaveValue('Original Name');
});

test('renaming a guild marks the save dirty and persists in the UI', async ({ page }) => {
  await load(page, [LEVEL]);
  await page.getByTestId('tab-guilds').click();
  await page.getByRole('button', { name: /Original Name/ }).click();

  await page.getByLabel('Guild name').fill('Renamed In Browser');
  await page.getByRole('button', { name: 'Rename', exact: true }).click();

  await expect(page.getByTestId('dirty')).toBeVisible();
  await expect(page.getByRole('button', { name: /Renamed In Browser/ })).toBeVisible();
});

/**
 * The multi-file path. Inventories need both files — the container ids live in the
 * player save, the contents in the level — and this is the code that had never run
 * in a browser before this suite existed.
 */
test('inventory is empty with only a level save', async ({ page }) => {
  await load(page, [LEVEL]);
  await page.getByTestId('tab-inventory').click();
  await expect(page.getByTestId('inventory-empty')).toBeVisible();
});

test('dropping level + player together resolves the inventory', async ({ page }) => {
  await load(page, [LEVEL, PLAYER]);

  await page.getByTestId('tab-inventory').click();
  await expect(page.getByTestId('inventory-empty')).toHaveCount(0);
  await expect(page.getByTestId('inventory-summary')).toBeVisible();

  // The synthetic player's one container holds 5 Wood, joined across both files.
  await expect(page.getByRole('cell', { name: 'Wood' })).toBeVisible();
});

test('export produces a non-empty download', async ({ page }) => {
  await load(page, [LEVEL]);

  const download = page.waitForEvent('download');
  await page.getByTestId('download').click();
  const file = await download;

  expect(file.suggestedFilename()).toBe('Level.sav');
  const stream = await file.createReadStream();
  const chunks: Buffer[] = [];
  for await (const chunk of stream) chunks.push(chunk as Buffer);
  expect(Buffer.concat(chunks).byteLength).toBeGreaterThan(0);
});

test('a garbage file surfaces a typed error rather than hanging', async ({ page }) => {
  await page.getByTestId('file-input').setInputFiles({
    name: 'broken.sav',
    mimeType: 'application/octet-stream',
    buffer: Buffer.from('this is not a palworld save'),
  });
  await expect(page.getByTestId('error-code')).toHaveText('container_decode_failed');
});
