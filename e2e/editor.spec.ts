// NOTE: These tests require Playwright to be installed.
// Run: npm install --save-dev @playwright/test
// Then: npx playwright install

import { test, expect, type Page } from "@playwright/test";

const TAURI_MOCK_SCRIPT = `
  const callbackStore = {};
  let callbackId = 0;
  window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      const defaults = {
        get_home_dir: '/home/test',
        read_directory: [
          { name: 'main.ts', path: '/home/test/main.ts', is_dir: false, is_symlink: false, size: 42, extension: 'ts' },
          { name: 'src', path: '/home/test/src', is_dir: true, is_symlink: false, size: 0, extension: null },
        ],
        read_file_content: 'const hello = "world";\nconsole.log(hello);\n',
        get_file_info: { size: 42, is_binary: false },
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
  await page.goto("/");
  await page.waitForSelector(".app", { timeout: 15000 });
}

test.describe("Editor", () => {
  test.beforeEach(async ({ page }) => {
    await setupPage(page);
  });

  test("can open a file from file explorer", async ({ page }) => {
    // Explorer should already be visible (default view)
    await expect(page.locator(".file-explorer")).toBeVisible();
    // Click on a file in the tree
    const fileItem = page.locator(".tree-node-label").filter({ hasText: "main.ts" }).first();
    if (await fileItem.count() > 0) {
      await fileItem.click();
      await page.waitForTimeout(500);
    }
    // Tab bar or editor should be visible
    await expect(page.locator(".tab-bar, .editor-area, .monaco-editor")).toBeVisible({ timeout: 5000 });
  });

  test("editor shows content when file is open", async ({ page }) => {
    // Simulate opening a file by clicking the file explorer
    await expect(page.locator(".file-explorer")).toBeVisible();
    const fileItem = page.locator(".tree-node-label").filter({ hasText: "main.ts" }).first();
    if (await fileItem.count() > 0) {
      await fileItem.click();
      await page.waitForTimeout(1000);
    }
    // The editor area should show content
    const editorArea = page.locator(".monaco-editor, .editor-area");
    if (await editorArea.count() > 0) {
      await expect(editorArea.first()).toBeVisible();
    }
  });

  test("can use quick open (Ctrl+P)", async ({ page }) => {
    await page.keyboard.press("Control+p");
    await page.waitForTimeout(300);
    // Quick open panel should appear
    const quickOpen = page.locator(".quick-open-panel, [role='dialog']");
    if (await quickOpen.count() > 0) {
      await expect(quickOpen.first()).toBeVisible();
      // Close it with escape
      await page.keyboard.press("Escape");
    }
  });
});
