#!/usr/bin/env node

const fs = require("fs");
const path = require("path");

const rootDir = path.resolve(__dirname, "..");
const quiet = process.argv.includes("--quiet");

const targetRoots = [
  path.join(rootDir, "src-tauri", "target"),
  path.join(rootDir, "cli", "target"),
  path.join(rootDir, "native", "target"),
];

const removableDirectories = [
  ["debug", "incremental"],
  ["release", "incremental"],
  ["debug", "build"],
  ["release", "build"],
  ["debug", "examples"],
  ["release", "examples"],
];

const removableTopLevelPatterns = [
  /^app_lib\.(lib|pdb|exp|ilk)$/i,
  /^libapp_lib\.rlib$/i,
  /^shadow_ide\.pdb$/i,
  /^shadowai\.pdb$/i,
  /^shadow_editor\.pdb$/i,
];

function log(message) {
  if (!quiet) {
    console.log(message);
  }
}

function removePath(targetPath) {
  if (!fs.existsSync(targetPath)) {
    return 0;
  }
  const stats = fs.statSync(targetPath);
  let removedBytes = 0;

  if (stats.isDirectory()) {
    for (const entry of fs.readdirSync(targetPath)) {
      removedBytes += removePath(path.join(targetPath, entry));
    }
    fs.rmSync(targetPath, { recursive: true, force: true });
    return removedBytes;
  }

  removedBytes = stats.size;
  fs.rmSync(targetPath, { force: true });
  return removedBytes;
}

function pruneTargetRoot(targetRoot) {
  let removedBytes = 0;

  if (!fs.existsSync(targetRoot)) {
    return removedBytes;
  }

  for (const segments of removableDirectories) {
    removedBytes += removePath(path.join(targetRoot, ...segments));
  }

  for (const profile of ["debug", "release"]) {
    const profileDir = path.join(targetRoot, profile);
    if (!fs.existsSync(profileDir)) {
      continue;
    }
    for (const entry of fs.readdirSync(profileDir)) {
      const entryPath = path.join(profileDir, entry);
      if (removableTopLevelPatterns.some((pattern) => pattern.test(entry))) {
        removedBytes += removePath(entryPath);
      }
    }

    const depsDir = path.join(profileDir, "deps");
    if (!fs.existsSync(depsDir)) {
      continue;
    }
    for (const entry of fs.readdirSync(depsDir)) {
      if (/\.pdb$/i.test(entry) || /^app_lib\.(lib|pdb|exp|ilk)$/i.test(entry) || /^libapp_lib\.rlib$/i.test(entry)) {
        removedBytes += removePath(path.join(depsDir, entry));
      }
    }
  }

  return removedBytes;
}

let totalRemovedBytes = 0;
for (const targetRoot of targetRoots) {
  const removedBytes = pruneTargetRoot(targetRoot);
  totalRemovedBytes += removedBytes;
  if (removedBytes > 0) {
    log(`[prune-rust-targets] ${path.relative(rootDir, targetRoot)}: removed ${(removedBytes / (1024 * 1024)).toFixed(1)} MB`);
  }
}

if (totalRemovedBytes === 0) {
  log("[prune-rust-targets] nothing to prune");
} else {
  log(`[prune-rust-targets] total reclaimed ${(totalRemovedBytes / (1024 * 1024 * 1024)).toFixed(2)} GB`);
}
