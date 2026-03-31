#!/usr/bin/env bash
# fix-appimage-deps.sh — Pre-bundle hook for AppImage builds on Linux.
#
# Tauri's AppImage bundler uses linuxdeploy, which can fail if certain
# system libraries are missing or if the linuxdeploy binary itself isn't
# available. This script ensures the prerequisites are in place.
set -euo pipefail

# Only run on Linux
if [[ "$(uname -s)" != "Linux" ]]; then
    echo "[fix-appimage-deps] Not Linux, skipping."
    exit 0
fi

TOOLS_DIR="${HOME}/.local/share/shadow-ide/tools"
LINUXDEPLOY="${TOOLS_DIR}/linuxdeploy-x86_64.AppImage"
LINUXDEPLOY_URL="https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage"

# Ensure linuxdeploy is available (Tauri expects it on PATH or in specific locations)
if ! command -v linuxdeploy &>/dev/null && [[ ! -f "${LINUXDEPLOY}" ]]; then
    echo "[fix-appimage-deps] Downloading linuxdeploy..."
    mkdir -p "${TOOLS_DIR}"
    curl -fsSL "${LINUXDEPLOY_URL}" -o "${LINUXDEPLOY}"
    chmod +x "${LINUXDEPLOY}"
    echo "[fix-appimage-deps] Downloaded to ${LINUXDEPLOY}"
fi

# Add tools dir to PATH so Tauri can find linuxdeploy
if [[ -f "${LINUXDEPLOY}" ]]; then
    export PATH="${TOOLS_DIR}:${PATH}"
fi

# Verify required system libraries for AppImage (common missing deps)
MISSING_DEPS=()
for lib in libgtk-3.so libwebkit2gtk-4.1.so libayatana-appindicator3.so; do
    if ! ldconfig -p 2>/dev/null | grep -q "${lib}"; then
        MISSING_DEPS+=("${lib}")
    fi
done

if [[ ${#MISSING_DEPS[@]} -gt 0 ]]; then
    echo "[fix-appimage-deps] Warning: Missing system libraries (AppImage may not include them):"
    printf "  - %s\n" "${MISSING_DEPS[@]}"
    echo "  Install them with your package manager if the AppImage fails to run."
fi

echo "[fix-appimage-deps] Pre-bundle checks complete."
