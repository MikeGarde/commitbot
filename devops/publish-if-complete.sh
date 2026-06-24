#!/usr/bin/env bash

set -euo pipefail

TAG="${1:?usage: publish-if-complete.sh <tag>}"

EXPECTED=(
  "commitbot-${TAG}-aarch64-apple-darwin.tar.gz"
  "commitbot-${TAG}-x86_64-apple-darwin.tar.gz"
  "commitbot-${TAG}-aarch64-unknown-linux-gnu.tar.gz"
  "commitbot-${TAG}-x86_64-unknown-linux-gnu.tar.gz"
  "commitbot-${TAG}-aarch64-unknown-linux-musl.tar.gz"
  "commitbot-${TAG}-x86_64-unknown-linux-musl.tar.gz"
  "commitbot-${TAG}-x86_64-pc-windows-gnu.zip"
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
if gh workflow run update-formula.yml \
  --repo MikeGarde/homebrew-tap \
  -f formula=commitbot \
  -f repo=MikeGarde/commitbot \
  -f tag="${TAG}"; then
  echo "Homebrew tap update dispatched."
else
  echo "Warning: could not dispatch Homebrew tap update. Trigger it manually with:"
  echo "  gh workflow run update-formula.yml --repo MikeGarde/homebrew-tap -f formula=commitbot -f repo=MikeGarde/commitbot -f tag=${TAG}"
fi
