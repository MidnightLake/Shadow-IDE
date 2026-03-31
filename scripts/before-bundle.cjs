#!/usr/bin/env node
// Cross-platform before-bundle script.
// 1. Builds the shadowai CLI and stages the binary for bundling.
// 2. On Linux, runs fix-appimage-deps.sh.

const { execSync } = require('child_process');
const os = require('os');
const path = require('path');
const fs = require('fs');

const rootDir = path.resolve(__dirname, '..');
const cliDir = path.join(rootDir, 'cli');
const cliBinDir = path.join(rootDir, 'cli-bin');

// Build the shadowai CLI
const isWindows = os.platform() === 'win32';
const binaryName = isWindows ? 'shadowai.exe' : 'shadowai';

try {
  console.log('[before-bundle] Building shadowai CLI...');
  execSync('cargo build --release', { cwd: cliDir, stdio: 'inherit' });

  // Stage the binary for Tauri resource bundling
  fs.mkdirSync(cliBinDir, { recursive: true });
  const src = path.join(cliDir, 'target', 'release', binaryName);
  const dest = path.join(cliBinDir, binaryName);
  if (fs.existsSync(src)) {
    fs.copyFileSync(src, dest);
    console.log(`[before-bundle] Staged ${binaryName} for bundling`);
  } else {
    console.warn(`[before-bundle] CLI binary not found at ${src}`);
  }
} catch (e) {
  console.warn('[before-bundle] CLI build failed (non-fatal):', e.message);
}

// Linux: fix AppImage deps
if (os.platform() === 'linux') {
  const script = path.join(__dirname, 'fix-appimage-deps.sh');
  try {
    execSync(`bash "${script}"`, { stdio: 'inherit' });
  } catch (e) {
    console.warn('[before-bundle] fix-appimage-deps.sh failed (non-fatal):', e.message);
  }
}
