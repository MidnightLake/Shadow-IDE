#!/usr/bin/env node

const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const rootDir = path.resolve(__dirname, "..");
const isWindows = process.platform === "win32";
const tauriBinary = path.join(rootDir, "node_modules", ".bin", isWindows ? "tauri.cmd" : "tauri");
const pruneScript = path.join(__dirname, "prune-rust-targets.cjs");
const args = process.argv.slice(2);
const primaryCommand = args[0] ?? "";
const shouldPrune = ["dev", "build"].includes(primaryCommand);

if (!fs.existsSync(tauriBinary)) {
  console.error(`[tauri-wrapper] Tauri CLI not found at ${tauriBinary}`);
  process.exit(1);
}

function runPrune(label) {
  const result = spawnSync(process.execPath, [pruneScript, "--quiet"], {
    cwd: rootDir,
    stdio: "inherit",
  });
  if ((result.status ?? 0) !== 0) {
    console.warn(`[tauri-wrapper] prune step ${label} exited with ${result.status ?? 0}`);
  }
}

if (shouldPrune && process.env.SHADOWIDE_SKIP_PRUNE !== "1") {
  runPrune("before");
}

const result = spawnSync(tauriBinary, args, {
  cwd: rootDir,
  stdio: "inherit",
  shell: isWindows,
});

if (shouldPrune && process.env.SHADOWIDE_SKIP_PRUNE !== "1") {
  runPrune("after");
}

if (typeof result.status === "number") {
  process.exit(result.status);
}

if (result.error) {
  console.error(result.error);
  process.exit(1);
}
