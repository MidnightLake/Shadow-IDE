#!/usr/bin/env bash
# fix-appimage-deps.sh
#
# Patches the Tauri-cached linuxdeploy GTK plugin for two issues on Fedora 45:
#
# 1. Missing librsvg2-devel: Plugin calls exit 1 when .pc file absent.
# 2. Old strip binary: linuxdeploy's bundled strip can't handle .relr.dyn
#    ELF sections. Patch GTK plugin to use system strip or skip stripping.

set -euo pipefail

TAURI_CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/tauri"
GTK_PLUGIN="$TAURI_CACHE/linuxdeploy-plugin-gtk.sh"

if [ ! -f "$GTK_PLUGIN" ]; then
  echo "[fix-appimage-deps] GTK plugin not cached yet; skipping."
  exit 0
fi

if grep -q "SHADOW_IDE_PATCHED_V2" "$GTK_PLUGIN"; then
  echo "[fix-appimage-deps] GTK plugin already patched (v2)."
  exit 0
fi

echo "[fix-appimage-deps] Patching GTK plugin at $GTK_PLUGIN ..."

python3 << 'PYEOF'
import sys, os

cache_dir = os.environ.get("XDG_CACHE_HOME", os.path.join(os.environ["HOME"], ".cache"))
path = os.path.join(cache_dir, "tauri", "linuxdeploy-plugin-gtk.sh")

with open(path, "r") as f:
    content = f.read()

# Remove old patch marker if present
content = content.replace("SHADOW_IDE_PATCHED", "SHADOW_IDE_OLD_PATCH")

# --- Patch 1: Make librsvg_libdir assignment conditional ---
old_librsvg = 'librsvg_libdir="$(get_pkgconf_variable "libdir" "librsvg-2.0")"'
new_librsvg = '''# SHADOW_IDE_PATCHED_V2: handle missing librsvg-2.0 gracefully
if "$PKG_CONFIG" --exists "librsvg-2.0" 2>/dev/null; then
    librsvg_libdir="$(get_pkgconf_variable "libdir" "librsvg-2.0")"
else
    echo "WARNING: librsvg-2.0 not found; skipping librsvg bundling."
    librsvg_libdir=""
fi'''

if old_librsvg in content:
    content = content.replace(old_librsvg, new_librsvg)

# Remove librsvg from FIND_ARRAY when libdir is empty
old_find = '    "$librsvg_libdir" "librsvg-*.so*"\n'
if old_find in content:
    content = content.replace(old_find, '')
    old_close = 'LIBRARIES=()'
    new_close = '''# Conditionally add librsvg
if [ -n "$librsvg_libdir" ]; then
    FIND_ARRAY+=("$librsvg_libdir" "librsvg-*.so*")
fi
LIBRARIES=()'''
    content = content.replace(old_close, new_close, 1)

# --- Patch 2: Replace strip calls with system strip or no-op ---
# The GTK plugin calls "strip" on bundled libs, but linuxdeploy's bundled
# strip is too old for .relr.dyn sections. Replace with system strip,
# and ignore failures (strip is optional for functionality).
if 'Calling strip on library' in content:
    # Find and patch the strip command invocation
    # The plugin typically does: strip "$file" or strip --strip-unneeded "$file"
    # Replace any "strip " call with "$(which strip 2>/dev/null || true) "
    # and add "|| true" to ignore failures
    import re
    # Match lines like: strip "$lib" or strip --strip-unneeded "$lib"
    content = re.sub(
        r'(\s+)(strip\s)',
        r'\1/usr/bin/strip ',
        content
    )
    # Make strip failures non-fatal
    content = re.sub(
        r'(ERROR: Strip call failed.*)',
        r'\1',
        content
    )
    # Find the function that calls strip and make it non-fatal
    # Look for pattern like: strip "$1" and make it strip "$1" || true
    content = re.sub(
        r'(/usr/bin/strip\s+(?:--strip-unneeded\s+)?"[^"]*")\s*$',
        r'\1 2>/dev/null || true',
        content,
        flags=re.MULTILINE
    )
    # Also handle: strip "$1" without quotes
    content = re.sub(
        r'(/usr/bin/strip\s+\$\{?\w+\}?)\s*$',
        r'\1 2>/dev/null || true',
        content,
        flags=re.MULTILINE
    )

with open(path, "w") as f:
    f.write(content)

print("[fix-appimage-deps] GTK plugin patched successfully (v2).")
PYEOF
