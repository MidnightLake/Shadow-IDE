// NOTE: These tests require Playwright to be installed.
// Run: npm install --save-dev @playwright/test
// Then: npx playwright install
//
// Visual regression tests use screenshot comparison.
// Run `npx playwright test --update-snapshots` once to generate baselines.

import { test, expect, type Page } from "@playwright/test";

const TAURI_MOCK_SCRIPT = `
  const callbackStore = {};
  let callbackId = 0;
  window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      const defaults = {
        get_home_dir: '/home/test',
        read_directory: [
          { name: 'index.ts', path: '/home/test/index.ts', is_dir: false, is_symlink: false, size: 120, extension: 'ts' },
        ],
        read_file_content: 'function greet(name: string): string {\\n  return "Hello, " + name;\\n}\\n',
        get_file_info: { size: 120, is_binary: false },
        project_list_recent: [],
        project_load_state: null,
        project_load_config: null,
        project_save_state: null,
        project_save_config: null,
        ai_check_connection: false,
        detect_shell: 'bash',
        list_terminals: [],
        create_terminal: 'term-1',
        resize_terminal: null,
        write_terminal: null,
        remote_get_info: { running: false, port: 9876, local_ip: '127.0.0.1', connected_clients: [] },
        remote_list_devices: [],
        remote_check_cert_expiry: 365,
        remote_detect_network: { local_ip: '127.0.0.1', tailscale_ip: null, tailscale_hostname: null, wireguard_ip: null },
        remote_update_state: null,
        scan_todos: [],
        git_is_repo: false,
        lsp_detect_servers: [],
        rag_get_stats: { files_indexed: 0, total_chunks: 0, last_index_time: '' },
        token_get_cache_stats: { entries: 0, hit_rate: 0.0 },
      };
      const key = cmd.replace('plugin:event|', '');
      if (key === 'listen') return Promise.resolve(callbackId);
      return Promise.resolve(defaults[key] !== undefined ? defaults[key] : null);
    },
    convertFileSrc: (path) => path,
    metadata: { currentWebview: { label: 'main' }, currentWindow: { label: 'main' } },
    transformCallback: (fn, once) => {
      const id = ++callbackId;
      callbackStore['_' + id] = fn;
      return id;
    },
  };
`;

async function setupPage(page: Page) {
  page.on("pageerror", () => {});
  await page.addInitScript({ content: TAURI_MOCK_SCRIPT });
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await page.waitForSelector(".app", { timeout: 15000 });
  // Wait for initial render to settle
  await page.waitForTimeout(500);
}

test.describe("Visual Regression", () => {
  test("whole app in Dark theme matches baseline", async ({ page }) => {
    await setupPage(page);
    // Dark theme is the default
    await expect(page).toHaveScreenshot("app-dark-theme.png", { threshold: 0.1 });
  });

  test("whole app in Light theme matches baseline", async ({ page }) => {
    await setupPage(page);
    // Switch to light theme via localStorage injection
    await page.evaluate(() => {
      localStorage.setItem("shadowide-theme", "light");
      window.location.reload();
    });
    await page.waitForSelector(".app", { timeout: 15000 });
    await page.waitForTimeout(500);
    await expect(page).toHaveScreenshot("app-light-theme.png", { threshold: 0.1 });
  });

  test("editor with code open matches baseline", async ({ page }) => {
    await setupPage(page);
    // Open a file from the explorer
    const fileItem = page.locator(".tree-node-label").first();
    if (await fileItem.count() > 0) {
      await fileItem.click();
      await page.waitForTimeout(1000);
    }
    await expect(page).toHaveScreenshot("app-editor-open.png", { threshold: 0.1 });
  });
});
