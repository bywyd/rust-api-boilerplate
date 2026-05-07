#!/usr/bin/env bash
# build-release.sh — Build all binaries in release mode and produce a versioned
# distributable zip for Linux x86_64.
#
# Usage:
#   bash scripts/build-release.sh
#   bash scripts/build-release.sh --target x86_64-unknown-linux-musl
#
# Output:
#   dist/<name>-<version>-linux-x86_64.zip
#     api
#     worker
#     updater
#     installer
#     uninstaller
#     config/default.yaml

set -euo pipefail

# Read name & version from Cargo.toml
CARGO_NAME=$(grep '^name' Cargo.toml | head -1 | sed 's/.*= *"\(.*\)"/\1/')
CARGO_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*= *"\(.*\)"/\1/')
TARGET_TRIPLE="${CARGO_BUILD_TARGET:-x86_64-unknown-linux-gnu}"
ARCH_SUFFIX="linux-x86_64"

BUNDLE_NAME="${CARGO_NAME}-${CARGO_VERSION}-${ARCH_SUFFIX}"
DIST_DIR="dist/${BUNDLE_NAME}"

echo "==> Building ${CARGO_NAME} v${CARGO_VERSION} (${TARGET_TRIPLE})"

# Compile
if [ -n "${CARGO_BUILD_TARGET:-}" ]; then
    cargo build --release --target "${TARGET_TRIPLE}"
    RELEASE_DIR="target/${TARGET_TRIPLE}/release"
else
    cargo build --release
    RELEASE_DIR="target/release"
fi

# Assemble bundle
echo "==> Assembling bundle in ${DIST_DIR}/"
rm -rf "${DIST_DIR}"
mkdir -p "${DIST_DIR}/config"

for BIN in api worker updater installer uninstaller; do
    SRC="${RELEASE_DIR}/${BIN}"
    if [ -f "${SRC}" ]; then
        cp "${SRC}" "${DIST_DIR}/${BIN}"
        chmod +x "${DIST_DIR}/${BIN}"
        echo "    ${BIN}"
    else
        echo "    Warning: ${SRC} not found, skipping"
    fi
done

cp config/default.yaml "${DIST_DIR}/config/default.yaml"

# Zip
mkdir -p dist
ZIP_PATH="dist/${BUNDLE_NAME}.zip"
echo "==> Creating ${ZIP_PATH}"
(cd dist && zip -r "${BUNDLE_NAME}.zip" "${BUNDLE_NAME}/")

# Checksum
echo "==> Computing SHA-256"
if command -v sha256sum &>/dev/null; then
    sha256sum "${ZIP_PATH}" | tee "${ZIP_PATH}.sha256"
elif command -v shasum &>/dev/null; then
    shasum -a 256 "${ZIP_PATH}" | tee "${ZIP_PATH}.sha256"
fi

# Manifests
PUBLISHED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
SHA256=$(cut -d' ' -f1 "${ZIP_PATH}.sha256" 2>/dev/null || echo '<SHA256_HERE>')

LATEST_PATH="dist/latest-linux.json"
VERSIONED_PATH="dist/v${CARGO_VERSION}-linux.json"

for MANIFEST_OUT in "${LATEST_PATH}" "${VERSIONED_PATH}"; do
    cat > "${MANIFEST_OUT}" <<MANIFEST_EOF
{
  "version": "${CARGO_VERSION}",
  "release_notes": "",
  "published_at": "${PUBLISHED_AT}",
  "platform": "${ARCH_SUFFIX}",
  "download_url": "https://YOUR_CDN/releases/v${CARGO_VERSION}/${BUNDLE_NAME}.zip",
  "checksum_sha256": "${SHA256}",
  "binary_name": "api"
}
MANIFEST_EOF
done

echo ""
echo "Release bundle         : ${ZIP_PATH}"
echo "Manifest (latest)      : ${LATEST_PATH}"
echo "Manifest (versioned)   : ${VERSIONED_PATH}"
echo ""
cat "${LATEST_PATH}"
