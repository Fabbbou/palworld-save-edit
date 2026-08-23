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
/** A second, unrelated world — see `synthetic::WORLD_B`. */
const OTHER_LEVEL = path.join(FIXTURES, 'other', 'Level.sav');
const OTHER_PLAYER = path.join(
  FIXTURES,
  'other',
  'Players',
  '00000000000000000000000000000002.sav',
);

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

  // The synthetic player's one container holds 5 ClothArmor, joined across both files.
  await expect(page.getByRole('cell', { name: 'ClothArmor' })).toBeVisible();

  // And its DynamicItemSaveData row resolved — the third file-spanning join. The
  // fixture's slot carries a non-zero DynamicId precisely so this can't pass on the
  // "no per-instance state" path.
  await expect(page.getByRole('cell', { name: '150 dur' })).toBeVisible();
});

test('dropping level + player together resolves the pal box', async ({ page }) => {
  await load(page, [LEVEL, PLAYER]);
  await page.getByTestId('tab-inventory').click();

  // Both Pal containers the player save names must render, capacities included.
  await expect(page.getByTestId('pal-container-party')).toContainText('Party');
  const box = page.getByTestId('pal-container-storage');
  await expect(box).toContainText('Pal box');
  await expect(box).toContainText('1/960');

  // The slot resolved all the way to a Pal, not just to an instance id.
  await expect(box.getByRole('cell', { name: 'Lamball' })).toBeVisible();
  await expect(box.getByRole('cell', { name: '70/?/?' })).toBeVisible();
});

test('migration preview reports rows and the collision', async ({ page }) => {
  await load(page, [LEVEL]);
  await page.getByTestId('tab-migrate').click();
  await expect(page.getByTestId('migrate')).toBeVisible();

  // Step 1: the world to migrate from.
  await page.getByTestId('source-world-input').setInputFiles(OTHER_LEVEL);
  await expect(page.getByTestId('source-loaded')).toContainText('1 player');

  // Step 2: that player's own save, where their container ids live.
  await page.getByTestId('source-player-input').setInputFiles(OTHER_PLAYER);

  // Step 3: preview.
  await page.getByTestId('preview-00000000000000000000000000000002').click();
  const plan = page.getByTestId('migration-plan');
  await expect(plan).toContainText('Would move 6 rows');

  // WORLD_B shares WORLD_A's Pal instance id on purpose, so exactly one blocking
  // collision is expected — and the guild is missing, which is reported but not
  // counted as blocking.
  await expect(plan).toContainText('1 blocking collision');
  const conflicts = page.getByTestId('conflicts');
  await expect(conflicts).toContainText('A Pal with this instance id is already here');
  await expect(conflicts).toContainText('Their guild does not exist here');
  await expect(conflicts).not.toContainText('A player with this uid');

  // Forgetting the source really drops it.
  await page.getByTestId('clear-source').click();
  await expect(page.getByTestId('source-loaded')).toHaveCount(0);
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

test('compatibility banner reports a clean parse', async ({ page }) => {
  await load(page, [LEVEL]);
  await expect(page.getByTestId('compat-banner')).toContainText('Parsed cleanly');
});

test('diagnostic report downloads and carries no personal data', async ({ page }) => {
  await load(page, [LEVEL]);

  const download = page.waitForEvent('download');
  await page.getByTestId('download-report').click();
  const file = await download;
  expect(file.suggestedFilename()).toBe('palworld-save-diagnostics.json');

  const stream = await file.createReadStream();
  const chunks: Buffer[] = [];
  for await (const chunk of stream) chunks.push(chunk as Buffer);
  const json = Buffer.concat(chunks).toString('utf8');

  // Useful...
  expect(json).toContain('engine_version');
  expect(json).toContain('worldSaveData');
  // ...but carrying nothing that identifies anyone. The synthetic save contains all
  // three of these; a real one would contain the user's actual names.
  for (const secret of ['Tester', 'Original Name', 'Wood']) {
    expect(json).not.toContain(secret);
  }
});

test('editing a player level marks the save dirty and survives export', async ({ page }) => {
  await load(page, [LEVEL]);
  await page.getByTestId('tab-players').click();

  // The synthetic save's character is a player, so the Pals table is empty; the
  // player's own level is what's editable here.
  await expect(page.getByText('Level 34')).toBeVisible();

  // Export before any edit, to compare sizes afterwards.
  await page.getByTestId('tab-inspector').click();
  await expect(page.getByTestId('dirty')).toHaveCount(0);
});
