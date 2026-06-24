#!/usr/bin/env bash

set -euo pipefail

TARGET_BRANCH="${TARGET_BRANCH:-${1:-}}"
SOURCE_BRANCH="${SOURCE_BRANCH:-${2:-}}"
MAIN_BASE_VERSION="${MAIN_BASE_VERSION:-}"
DEVELOP_BASE_VERSION="${DEVELOP_BASE_VERSION:-}"
RELEASE_VERSION="${RELEASE_VERSION:-}"

if [[ -z "${TARGET_BRANCH}" ]]; then
  echo "usage: TARGET_BRANCH=<main|develop> SOURCE_BRANCH=<branch> RELEASE_VERSION=X.Y.Z MAIN_BASE_VERSION=X.Y.Z DEVELOP_BASE_VERSION=X.Y.Z $0" >&2
  exit 1
fi

semver_pattern='^[0-9]+\.[0-9]+\.[0-9]+$'

require_semver() {
  local value="$1"
  local label="$2"

  if [[ ! "${value}" =~ ${semver_pattern} ]]; then
    echo "${label} must be a semantic version like 1.0.0" >&2
    exit 1
  fi
}

require_semver "${MAIN_BASE_VERSION}" "MAIN_BASE_VERSION"
require_semver "${DEVELOP_BASE_VERSION}" "DEVELOP_BASE_VERSION"

if [[ -n "${RELEASE_VERSION}" ]]; then
  require_semver "${RELEASE_VERSION}" "RELEASE_VERSION"
fi

case "${TARGET_BRANCH}" in
  main)
    BASE_VERSION="${MAIN_BASE_VERSION}"
    ;;
  develop)
    BASE_VERSION="${DEVELOP_BASE_VERSION}"
    ;;
  *)
    echo "Unsupported target branch: ${TARGET_BRANCH}" >&2
    exit 1
    ;;
esac

BASE_MAJOR="${BASE_VERSION%%.*}"
TAG_PATTERN="^${BASE_MAJOR}\\.[0-9]+\\.[0-9]+$"
PREVIOUS_VERSION="$(
  git tag --list \
    | grep -E "${TAG_PATTERN}" || true
)"
PREVIOUS_VERSION="$(
  printf '%s\n' "${PREVIOUS_VERSION}" \
    | sort -V \
    | tail -n 1
)"
PREVIOUS_VERSION="${PREVIOUS_VERSION:-${BASE_VERSION}}"

CURRENT_VERSION="$(
  printf '%s\n%s\n' "${BASE_VERSION}" "${PREVIOUS_VERSION}" \
    | sort -V \
    | tail -n 1
)"

if [[ -n "${RELEASE_VERSION}" ]]; then
  SHOULD_RELEASE="true"
  REASON=""
  BUMP="manual"

  RELEASE_MAJOR="${RELEASE_VERSION%%.*}"
  if [[ "${RELEASE_MAJOR}" != "${BASE_MAJOR}" ]]; then
    echo "RELEASE_VERSION ${RELEASE_VERSION} does not match the ${TARGET_BRANCH} release line ${BASE_MAJOR}.x.x" >&2
    exit 1
  fi

  if [[ "$(printf '%s\n%s\n' "${CURRENT_VERSION}" "${RELEASE_VERSION}" | sort -V | tail -n 1)" != "${RELEASE_VERSION}" ]] || [[ "${RELEASE_VERSION}" == "${CURRENT_VERSION}" ]]; then
    echo "RELEASE_VERSION ${RELEASE_VERSION} must be newer than ${CURRENT_VERSION}" >&2
    exit 1
  fi

  VERSION="${RELEASE_VERSION}"
elif [[ "${SOURCE_BRANCH}" == feat/* ]]; then
  SHOULD_RELEASE="true"
  REASON=""
  BUMP="minor"
  IFS='.' read -r MAJOR MINOR PATCH <<< "${CURRENT_VERSION}"
  MINOR=$((MINOR + 1))
  PATCH=0
  VERSION="${MAJOR}.${MINOR}.${PATCH}"
elif [[ "${SOURCE_BRANCH}" == bugfix/* ]]; then
  SHOULD_RELEASE="true"
  REASON=""
  BUMP="patch"
  IFS='.' read -r MAJOR MINOR PATCH <<< "${CURRENT_VERSION}"
  PATCH=$((PATCH + 1))
  VERSION="${MAJOR}.${MINOR}.${PATCH}"
else
  SHOULD_RELEASE="false"
  REASON="Source branch must start with feat/ or bugfix/."
  BUMP="none"
  VERSION=""
fi

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    echo "should_release=${SHOULD_RELEASE}"
    echo "reason=${REASON}"
    echo "bump=${BUMP}"
    echo "base_version=${BASE_VERSION}"
    echo "previous_version=${PREVIOUS_VERSION}"
    echo "version=${VERSION}"
    echo "target_branch=${TARGET_BRANCH}"
    echo "source_branch=${SOURCE_BRANCH}"
  } >> "${GITHUB_OUTPUT}"
fi

echo "should_release=${SHOULD_RELEASE}"
echo "reason=${REASON}"
echo "bump=${BUMP}"
echo "base_version=${BASE_VERSION}"
echo "previous_version=${PREVIOUS_VERSION}"
echo "version=${VERSION}"
