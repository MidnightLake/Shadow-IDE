import { test, expect, type Page } from "@playwright/test";

// Mock Tauri internals so the app renders without the Rust backend.
const TAURI_MOCK_SCRIPT = `
  const callbackStore = {};
  let callbackId = 0;
  window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      const defaults = {
        get_home_dir: '/home/test',
        read_directory: [],
        project_list_recent: [],
        project_load_state: null,
        project_load_config: null,
        project_save_state: null,
        project_save_config: null,
        ai_check_connection: false,
        ai_detect_providers: [],
        ai_get_models: [],
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
      // Handle plugin:event|listen and similar namespaced commands
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

test.describe("App Shell", () => {
  test.beforeEach(async ({ page }) => {
    await setupPage(page);
  });

  test("renders activity bar and status bar", async ({ page }) => {
    await expect(page.locator(".activity-bar")).toBeVisible();
    await expect(page.locator(".status-bar")).toBeVisible();
  });

  test("status bar contains ShadowIDE", async ({ page }) => {
    const text = await page.locator(".status-bar").textContent();
    expect(text).toContain("ShadowIDE");
  });

  test("activity bar has navigation buttons", async ({ page }) => {
    const buttons = page.locator(".activity-bar .activity-btn");
    const count = await buttons.count();
    expect(count).toBeGreaterThanOrEqual(6);
  });

  test("sidebar is visible with default explorer view", async ({ page }) => {
    await expect(page.locator(".sidebar")).toBeVisible();
    await expect(page.locator(".file-explorer")).toBeVisible();
  });

  test("clicking AI button shows AI chat", async ({ page }) => {
    await page.locator(".activity-btn").nth(1).click();
    await page.waitForTimeout(200);
    await expect(page.locator(".ai-chat")).toBeVisible();
  });

  test("clicking Search button shows search panel", async ({ page }) => {
    await page.locator(".activity-btn").nth(3).click();
    await page.waitForTimeout(200);
    await expect(page.locator(".search-panel")).toBeVisible();
    await expect(page.locator("text=SEARCH")).toBeVisible();
  });

  test("clicking Diagnostics button shows todo panel", async ({ page }) => {
    await page.locator(".activity-btn").nth(2).click();
    await page.waitForTimeout(200);
    await expect(page.locator(".todo-panel")).toBeVisible();
    await expect(page.locator("text=DIAGNOSTICS")).toBeVisible();
  });

  test("clicking Remote button shows remote settings", async ({ page }) => {
    await page.locator(".activity-btn").nth(4).click();
    await page.waitForTimeout(200);
    await expect(page.locator(".remote-settings")).toBeVisible();
    await expect(page.locator("text=REMOTE ACCESS")).toBeVisible();
  });

  test("clicking Settings button shows settings panel", async ({ page }) => {
    await page.locator(".activity-btn[title='Settings']").click();
    await page.waitForTimeout(200);
    await expect(page.locator(".settings-panel")).toBeVisible();
    await expect(page.locator("text=SETTINGS")).toBeVisible();
    await expect(page.locator("text=Appearance")).toBeVisible();
    await expect(page.locator("text=OLED Mode")).toBeVisible();
    await expect(page.locator("text=Editor")).toBeVisible();
    await expect(page.locator("text=Keyboard Shortcuts")).toBeVisible();
  });

  test("OLED toggle changes html data-oled attribute", async ({ page }) => {
    await page.locator(".activity-btn[title='Settings']").click();
    await page.waitForTimeout(200);

    const oledRow = page.locator("text=OLED Mode").locator("..");
    const toggle = oledRow.locator("input[type='checkbox']");
    await toggle.click();

    const oledVal = await page.locator("html").getAttribute("data-oled");
    expect(oledVal).toBe("true");
  });

  test("sidebar position changes layout class", async ({ page }) => {
    await page.locator(".activity-btn[title='Settings']").click();
    await page.waitForTimeout(200);

    const select = page.locator(".settings-select");
    await select.selectOption("right");
    await page.waitForTimeout(200);

    await expect(page.locator(".app")).toHaveClass(/sidebar-right/);
  });

  test("sidebar position top changes layout class", async ({ page }) => {
    await page.locator(".activity-btn[title='Settings']").click();
    await page.waitForTimeout(200);

    const select = page.locator(".settings-select");
    await select.selectOption("top");
    await page.waitForTimeout(200);

    await expect(page.locator(".app")).toHaveClass(/sidebar-top/);
  });

  test("sidebar position bottom changes layout class", async ({ page }) => {
    await page.locator(".activity-btn[title='Settings']").click();
    await page.waitForTimeout(200);

    const select = page.locator(".settings-select");
    await select.selectOption("bottom");
    await page.waitForTimeout(200);

    await expect(page.locator(".app")).toHaveClass(/sidebar-bottom/);
  });
});

test.describe("Keyboard Shortcuts", () => {
  test.beforeEach(async ({ page }) => {
    await setupPage(page);
  });

  test("Ctrl+Shift+A opens AI chat", async ({ page }) => {
    await page.keyboard.press("Control+Shift+A");
    await page.waitForTimeout(300);
    await expect(page.locator(".ai-chat")).toBeVisible();
  });

  test("Ctrl+Shift+F opens search", async ({ page }) => {
    await page.keyboard.press("Control+Shift+F");
    await page.waitForTimeout(300);
    await expect(page.locator(".search-panel")).toBeVisible();
  });

  test("Ctrl+Shift+T opens diagnostics", async ({ page }) => {
    await page.keyboard.press("Control+Shift+T");
    await page.waitForTimeout(300);
    await expect(page.locator(".todo-panel")).toBeVisible();
  });

  test("Ctrl+Shift+R opens remote settings", async ({ page }) => {
    await page.keyboard.press("Control+Shift+R");
    await page.waitForTimeout(300);
    await expect(page.locator(".remote-settings")).toBeVisible();
  });
});

test.describe("Search Panel Interaction", () => {
  test.beforeEach(async ({ page }) => {
    await setupPage(page);
    await page.locator(".activity-btn").nth(3).click();
    await page.waitForTimeout(200);
  });

  test("search input accepts text", async ({ page }) => {
    const input = page.locator(".search-input").first();
    await input.fill("test query");
    expect(await input.inputValue()).toBe("test query");
  });

  test("replace toggle shows replace input", async ({ page }) => {
    await page.locator("[title='Toggle Replace']").click();
    const replaceInput = page.locator(".search-input").nth(1);
    await expect(replaceInput).toBeVisible();
  });

  test("extension filter input works", async ({ page }) => {
    const extInput = page.locator(".search-ext-input");
    await extInput.fill("ts,rs");
    expect(await extInput.inputValue()).toBe("ts,rs");
  });
});
