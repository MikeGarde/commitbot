#!/bin/zsh
set -euo pipefail

#
# Config
#
DIST_DIR="dist"

STEP=${1:?usage: release.sh [major|minor|patch|X.Y.Z]}

#
# Pre-flight checks
#
if ! command -v task >/dev/null 2>&1; then
  echo "Error: 'task' command not found. Install Taskfile runner first."
  exit 1
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "Error: 'gh' CLI not found. Install GitHub CLI (gh) first."
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "Error: 'jq' not found. Install jq first."
  exit 1
fi

REPO_SLUG=$(gh repo view --json nameWithOwner -q .nameWithOwner)

BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [ "$BRANCH" != "main" ]; then
  echo "Error: releases may only be created from 'main' (current: $BRANCH)."
  exit 1
fi

# Ensure working tree is clean
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "Error: working tree is not clean. Commit or stash changes before releasing."
  exit 1
fi

#
# Read current version from Cargo.toml
#
CURRENT_VERSION=$(
  grep -E '^version = "[0-9]+\.[0-9]+\.[0-9]+"' Cargo.toml \
  | head -n1 \
  | sed -E 's/version = "([^"]+)"/\1/'
)

if [ -z "$CURRENT_VERSION" ]; then
  echo "Error: could not determine current version from Cargo.toml"
  exit 1
fi

MAJOR=$(echo "$CURRENT_VERSION" | cut -d. -f1)
MINOR=$(echo "$CURRENT_VERSION" | cut -d. -f2)
PATCH=$(echo "$CURRENT_VERSION" | cut -d. -f3)
LATEST=true

#
# Compute new version
#
case "$STEP" in
  major)
    MAJOR=$((MAJOR+1))
    MINOR=0
    PATCH=0
    ;;
  minor)
    MINOR=$((MINOR+1))
    PATCH=0
    ;;
  patch)
    PATCH=$((PATCH+1))
    ;;
  *)
    if [[ "$STEP" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
      MAJOR=$(echo "$STEP" | cut -d. -f1)
      MINOR=$(echo "$STEP" | cut -d. -f2)
      PATCH=$(echo "$STEP" | cut -d. -f3)
      LATEST=false
    else
      echo "Invalid step: $STEP"
      echo "Usage: release.sh [major|minor|patch|X.Y.Z]"
      exit 1
    fi
    ;;
esac

NEW_VERSION="$MAJOR.$MINOR.$PATCH"

#
# Join mode: tag already exists (e.g. GA created the release first)
#
if git rev-parse -q --verify "refs/tags/$NEW_VERSION" >/dev/null 2>&1; then
  echo "Tag $NEW_VERSION already exists. Joining existing draft release to upload Mac assets..."
  echo

  RELEASE_IS_DRAFT=$(gh release view "$NEW_VERSION" --json isDraft --jq .isDraft 2>/dev/null || echo "not_found")
  if [ "$RELEASE_IS_DRAFT" = "false" ]; then
    echo "Release $NEW_VERSION is already published. Nothing to do."
    exit 0
  fi
  if [ "$RELEASE_IS_DRAFT" = "not_found" ]; then
    echo "Error: tag $NEW_VERSION exists in git but no GitHub release was found."
    exit 1
  fi

  CARGO_VERSION=$(
    grep -E '^version = "[0-9]+\.[0-9]+\.[0-9]+"' Cargo.toml \
    | head -n1 \
    | sed -E 's/version = "([^"]+)"/\1/'
  )
  if [ "$CARGO_VERSION" != "$NEW_VERSION" ]; then
    echo "Error: Cargo.toml version ($CARGO_VERSION) does not match release version ($NEW_VERSION)."
    echo "Run 'git pull' to sync the release commit, then retry."
    exit 1
  fi

  echo "Building Mac binaries for $NEW_VERSION..."
  task setup
  task "build:release:${NEW_VERSION}"

  MAC_ASSETS=("$DIST_DIR"/commitbot-"$NEW_VERSION"-*apple-darwin*.tar.gz)
  if [ ${#MAC_ASSETS[@]} -eq 0 ]; then
    echo "Error: no Mac assets found in $DIST_DIR after build."
    exit 1
  fi

  echo "Uploading Mac assets to existing draft release $NEW_VERSION:"
  for a in "${MAC_ASSETS[@]}"; do
    echo "  - $a"
  done
  gh release upload "$NEW_VERSION" "${MAC_ASSETS[@]}" --clobber

  bash devops/publish-if-complete.sh "$NEW_VERSION"
  exit 0
fi

#
# Full release flow
#
echo "Current version: $CURRENT_VERSION"
echo "New version:     $NEW_VERSION"
echo "Branch:          $BRANCH"
echo

read -r -p "Proceed with release? (y/N): " CONFIRM
if [[ ! "$CONFIRM" =~ ^[Yy]$ ]]; then
  echo "Aborted."
  exit 1
fi

#
# Bump Cargo.toml with rollback on failure
#
CARGO_BAK="Cargo.toml.bak.$$"
cp Cargo.toml "$CARGO_BAK"

cleanup_on_error() {
  rc=$?
  echo "Error during release (exit code $rc). Restoring Cargo.toml..."
  if [ -f "$CARGO_BAK" ]; then
    mv "$CARGO_BAK" Cargo.toml
  fi
  exit $rc
}
trap cleanup_on_error INT TERM ERR

echo "Updating Cargo.toml to version $NEW_VERSION..."
perl -pi -e 's/^version = "[0-9]+\.[0-9]+\.[0-9]+"/version = "'"$NEW_VERSION"'"/' Cargo.toml

#
# Run tasks: setup, test, build
#
echo "Running task setup..."
task setup

echo "Running task build:release:${NEW_VERSION}..."
task "build:release:${NEW_VERSION}"

# If we got here, tasks succeeded – stop rollback trap
trap - INT TERM ERR
rm -f "$CARGO_BAK"

#
# Commit, tag, push
#
if git diff --quiet -- Cargo.toml; then
  echo "Warning: Cargo.toml did not change; nothing to commit."
else
  git commit Cargo.toml Cargo.lock -m "$NEW_VERSION"
fi

HASH=$(git rev-parse HEAD)

echo "Tagging $NEW_VERSION..."
git tag "$NEW_VERSION" "$HASH"

echo "Pushing main and tag..."
git push origin main
git push origin "refs/tags/$NEW_VERSION"

#
# Create draft GitHub release with Mac assets
#
if [ ! -d "$DIST_DIR" ]; then
  echo "Error: build artifacts directory '$DIST_DIR' not found."
  echo "Check task build:release:all"
  exit 1
fi

MAC_ASSETS=("$DIST_DIR"/commitbot-"$NEW_VERSION"-*apple-darwin*.tar.gz)
if [ ${#MAC_ASSETS[@]} -eq 0 ]; then
  echo "Error: no Mac assets found in '$DIST_DIR'."
  exit 1
fi

echo "Creating draft GitHub release $NEW_VERSION with Mac assets:"
for a in "${MAC_ASSETS[@]}"; do
  echo "  - $a"
done

NOTES_FILE="$(mktemp)"
COMBINED_NOTES_FILE="$(mktemp)"

cleanup_notes() {
  rm -f "$NOTES_FILE" "$COMBINED_NOTES_FILE"
}
trap cleanup_notes EXIT

gh api \
  -H "Accept: application/vnd.github+json" \
  "/repos/$REPO_SLUG/releases/generate-notes" \
  -f tag_name="$NEW_VERSION" \
  -f target_commitish="$HASH" \
  --jq .body > "$NOTES_FILE"

bash devops/render-release-notes.sh "$NEW_VERSION" "$REPO_SLUG" "$NOTES_FILE" > "$COMBINED_NOTES_FILE"

LATEST_ARGS=()
if [ "$LATEST" = true ]; then
  LATEST_ARGS+=(--latest)
fi

gh release create "$NEW_VERSION" \
  --draft \
  --fail-on-no-commits \
  "${LATEST_ARGS[@]}" \
  --notes-file "$COMBINED_NOTES_FILE" \
  --target "$HASH" \
  "${MAC_ASSETS[@]}"

#
# Dispatch release-assets workflow for Linux + Windows
#
echo
echo "Dispatching release-assets workflow for Linux and Windows builds..."

SOURCE_RUN_URL="local:$(hostname)"

gh workflow run release-assets.yml \
  --ref main \
  -f tag="$NEW_VERSION" \
  -f source_run_url="${SOURCE_RUN_URL}"

echo "Dispatched. Linux and Windows assets will be uploaded by GitHub Actions."
echo "The release will be published automatically once all assets are present."
echo

#
# Check if all assets happen to already be present (rare but possible)
#
bash devops/publish-if-complete.sh "$NEW_VERSION"

echo "Release $NEW_VERSION draft created successfully."
exit 0
