#!/usr/bin/env bash

set -euo pipefail

TAG="${1:?usage: publish-if-complete.sh <tag>}"

EXPECTED=(
  "commitbot-${TAG}-apple-darwin-aarch64.tar.gz"
  "commitbot-${TAG}-apple-darwin-x86_64.tar.gz"
  "commitbot-${TAG}-unknown-linux-gnu-aarch64.tar.gz"
  "commitbot-${TAG}-unknown-linux-gnu-x86_64.tar.gz"
  "commitbot-${TAG}-unknown-linux-musl-aarch64.tar.gz"
  "commitbot-${TAG}-unknown-linux-musl-x86_64.tar.gz"
  "commitbot-${TAG}-pc-windows-gnu-x86_64.zip"
)

ACTUAL="$(gh release view "${TAG}" --json assets --jq '[.assets[].name]')"

for asset in "${EXPECTED[@]}"; do
  if ! printf '%s' "${ACTUAL}" | jq -e --arg n "${asset}" 'any(.[]; . == $n)' > /dev/null 2>&1; then
    echo "Release not yet complete (missing: ${asset}). Skipping publish."
    exit 0
  fi
done

echo "All expected assets present. Publishing release ${TAG}..."
gh release edit "${TAG}" --draft=false
echo "Published."

echo "Triggering Homebrew tap formula update..."
if GH_TOKEN="${HOMEBREW_TAP_TOKEN:-}" gh workflow run update-formula.yml \
  --repo MikeGarde/homebrew-tap \
  -f formula=commitbot \
  -f repo=MikeGarde/commitbot \
  -f tag="${TAG}"; then
  echo "Homebrew tap update dispatched."
else
  echo "Warning: could not dispatch Homebrew tap update. Trigger it manually with:"
  echo "  gh workflow run update-formula.yml --repo MikeGarde/homebrew-tap -f formula=commitbot -f repo=MikeGarde/commitbot -f tag=${TAG}"
fi
