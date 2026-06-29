#!/usr/bin/env bash

set -euo pipefail

VERSION="${1:?usage: render-release-notes.sh <version> <repo> [notes-file]}"
REPO="${2:?usage: render-release-notes.sh <version> <repo> [notes-file]}"
NOTES_FILE="${3:-}"

asset_url() {
  local filename="$1"
  printf 'https://github.com/%s/releases/download/%s/%s' "${REPO}" "${VERSION}" "${filename}"
}

cat <<EOF
## Downloads

| OS | arm64 | x86_64 |
| --- | --- | --- |
| macOS | [Download]($(asset_url "commitbot-${VERSION}-aarch64-apple-darwin.tar.gz")) | [Download]($(asset_url "commitbot-${VERSION}-x86_64-apple-darwin.tar.gz")) |
| Ubuntu* | [Download]($(asset_url "commitbot-${VERSION}-aarch64-unknown-linux-gnu.tar.gz")) | [Download]($(asset_url "commitbot-${VERSION}-x86_64-unknown-linux-gnu.tar.gz")) |
| RHEL** | [Download]($(asset_url "commitbot-${VERSION}-aarch64-unknown-linux-musl.tar.gz")) | [Download]($(asset_url "commitbot-${VERSION}-x86_64-unknown-linux-musl.tar.gz")) |
| Windows*** | — | [Download]($(asset_url "commitbot-${VERSION}-x86_64-pc-windows-gnu.zip")) |

\* Ubuntu and compatible distributions like Debian, Mint, etc. that use glibc.
\** RHEL and compatible distributions like Amazon, Rocky, etc. that use musl instead of glibc.
\*** Windows x86_64 only; built with the GNU toolchain (mingw-w64).

EOF

if [[ -n "${NOTES_FILE}" && -f "${NOTES_FILE}" ]]; then
  cat "${NOTES_FILE}"
fi
