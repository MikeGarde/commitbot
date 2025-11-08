#!/bin/zsh
set -euo pipefail

# ---------------------------------------------------------
# Config
# ---------------------------------------------------------
DIST_DIR="dist"

STEP=${1:?usage: release.sh [major|minor|patch|X.Y.Z]}

# ---------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------
if ! command -v task >/dev/null 2>&1; then
  echo "Error: 'task' command not found. Install Taskfile runner first."
  exit 1
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "Error: 'gh' CLI not found. Install GitHub CLI (gh) first."
  exit 1
fi

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

# ---------------------------------------------------------
# Read current version from Cargo.toml
# ---------------------------------------------------------
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

# ---------------------------------------------------------
# Compute new version
# ---------------------------------------------------------
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

# Check that tag does not already exist
if git rev-parse -q --verify "refs/tags/$NEW_VERSION" >/dev/null 2>&1; then
  echo "Error: git tag $NEW_VERSION already exists."
  exit 1
fi

echo "Current version: $CURRENT_VERSION"
echo "New version:     $NEW_VERSION"
echo "Branch:          $BRANCH"
echo

read -r -p "Proceed with release? (y/N): " CONFIRM
if [[ ! "$CONFIRM" =~ ^[Yy]$ ]]; then
  echo "Aborted."
  exit 1
fi

# ---------------------------------------------------------
# Bump Cargo.toml with rollback on failure
# ---------------------------------------------------------
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

# ---------------------------------------------------------
# Run tasks: setup, test, build
# ---------------------------------------------------------
echo "Running task setup..."
task setup

echo "Running task test..."
task test

echo "Running task build:release:all..."
task "build:release:${NEW_VERSION}"

# If we got here, tasks succeeded – stop rollback trap
trap - INT TERM ERR
rm -f "$CARGO_BAK"

# ---------------------------------------------------------
# Commit, tag, push
# ---------------------------------------------------------
if git diff --quiet -- Cargo.toml; then
  echo "Warning: Cargo.toml did not change; nothing to commit."
else
  git commit Cargo.toml -m "$NEW_VERSION"
fi

HASH=$(git rev-parse HEAD)

echo "Tagging $NEW_VERSION..."
git tag "$NEW_VERSION" "$HASH"

echo "Pushing main and tag..."
git push origin main
git push origin "refs/tags/$NEW_VERSION"

# ---------------------------------------------------------
# Create GitHub release from artifacts in DIST_DIR
# ---------------------------------------------------------
if [ ! -d "$DIST_DIR" ]; then
  echo "Error: build artifacts directory '$DIST_DIR' not found."
  echo "Check task build:release:all"
  exit 1
fi

ASSETS=("$DIST_DIR"/*)
if [ ${#ASSETS[@]} -eq 0 ]; then
  echo "Error: no build artifacts found in '$DIST_DIR'."
  exit 1
fi

LATEST_FLAG=""
if [ "$LATEST" = true ]; then
  LATEST_FLAG="--latest"
fi

echo "Creating GitHub release $NEW_VERSION with assets:"
for a in "${ASSETS[@]}"; do
  echo "  - $a"
done

gh release create "$NEW_VERSION" \
  --fail-on-no-commits \
  --generate-notes \
  $LATEST_FLAG \
  --target "$HASH" \
  "${ASSETS[@]}"

echo "Release $NEW_VERSION created successfully."
exit 0
