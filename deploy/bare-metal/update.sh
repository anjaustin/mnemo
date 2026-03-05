#!/usr/bin/env bash
# ============================================================================
# Mnemo binary updater — bare-metal / VPS
#
# Downloads the latest (or specified) mnemo-server binary from GitHub Releases,
# verifies the SHA256 checksum, swaps it in place, and restarts the service.
#
# Usage:
#   ./update.sh              # update to latest release
#   ./update.sh 0.3.1        # update to a specific version
#
# Requirements:
#   curl, sha256sum, systemctl, sudo access
# ============================================================================

set -euo pipefail

REPO="anjaustin/mnemo"
BINARY_NAME="mnemo-server"
INSTALL_PATH="/usr/local/bin/${BINARY_NAME}"
SERVICE_NAME="mnemo"
TMP_DIR="$(mktemp -d)"

# Detect architecture
ARCH="$(uname -m)"
case "${ARCH}" in
  x86_64)   ASSET_ARCH="x86_64-unknown-linux-gnu" ;;
  aarch64)  ASSET_ARCH="aarch64-unknown-linux-gnu" ;;
  *)        echo "ERROR: unsupported architecture: ${ARCH}" >&2; exit 1 ;;
esac

# Resolve version
if [[ "${1:-}" == "" ]]; then
  echo "Fetching latest release version..."
  VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' | sed -E 's/.*"v?([^"]+)".*/\1/')"
  echo "Latest version: ${VERSION}"
else
  VERSION="${1#v}"  # strip leading 'v' if provided
fi

ASSET="${BINARY_NAME}-${ASSET_ARCH}"
RELEASE_URL="https://github.com/${REPO}/releases/download/v${VERSION}"

echo "Downloading ${ASSET} v${VERSION}..."
curl -fsSL -o "${TMP_DIR}/${BINARY_NAME}" "${RELEASE_URL}/${ASSET}"

echo "Downloading SHA256SUMS..."
curl -fsSL -o "${TMP_DIR}/SHA256SUMS" "${RELEASE_URL}/SHA256SUMS"

echo "Verifying checksum..."
(cd "${TMP_DIR}" && grep "${ASSET}" SHA256SUMS | sha256sum --check --strict)

echo "Installing ${INSTALL_PATH}..."
chmod +x "${TMP_DIR}/${BINARY_NAME}"
sudo mv "${TMP_DIR}/${BINARY_NAME}" "${INSTALL_PATH}"

rm -rf "${TMP_DIR}"

echo "Restarting ${SERVICE_NAME} service..."
sudo systemctl restart "${SERVICE_NAME}"

echo "Waiting for health check..."
sleep 3
if curl -fsSL http://127.0.0.1:8080/health | grep -q '"status":"ok"'; then
  echo "Update complete — mnemo-server v${VERSION} is running."
else
  echo "WARNING: health check did not return ok. Check: journalctl -u ${SERVICE_NAME} -n 50"
  exit 1
fi
