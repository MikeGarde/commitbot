#!/usr/bin/env bash

set -euo pipefail

TARGET_BRANCH="${TARGET_BRANCH:-${1:-}}"
RELEASE_VERSION="${RELEASE_VERSION:-${2:-}}"

if [[ -z "${TARGET_BRANCH}" ]]; then
  echo "usage: TARGET_BRANCH=<main|develop> RELEASE_VERSION=X.Y.Z $0" >&2
  exit 1
fi

case "${TARGET_BRANCH}" in
  main|develop) ;;
  *)
    echo "Unsupported target branch: ${TARGET_BRANCH}" >&2
    exit 1
    ;;
esac

semver_pattern='^[0-9]+\.[0-9]+\.[0-9]+$'

if [[ ! "${RELEASE_VERSION}" =~ ${semver_pattern} ]]; then
  echo "RELEASE_VERSION must be a semantic version like 1.0.0" >&2
  exit 1
fi

# Newest released version (empty when no semver tags exist yet).
PREVIOUS_VERSION="$(
  git tag --list \
    | grep -E "${semver_pattern}" || true
)"
PREVIOUS_VERSION="$(
  printf '%s\n' "${PREVIOUS_VERSION}" \
    | sort -V \
    | tail -n 1
)"

# Baseline for the next version: the newest of the latest tag and the version
# currently in Cargo.toml, so the requested version can never move backwards.
CARGO_VERSION="$(
  sed -nE 's/^version = "([0-9]+\.[0-9]+\.[0-9]+)"/\1/p' Cargo.toml | head -n1
)"

CURRENT_VERSION="$(
  printf '%s\n%s\n' "${PREVIOUS_VERSION}" "${CARGO_VERSION}" \
    | sort -V \
    | tail -n 1
)"

newest="$(printf '%s\n%s\n' "${CURRENT_VERSION}" "${RELEASE_VERSION}" | sort -V | tail -n 1)"
if [[ "${newest}" != "${RELEASE_VERSION}" ]] || [[ "${RELEASE_VERSION}" == "${CURRENT_VERSION}" ]]; then
  echo "RELEASE_VERSION ${RELEASE_VERSION} must be newer than ${CURRENT_VERSION}" >&2
  exit 1
fi

VERSION="${RELEASE_VERSION}"

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    echo "previous_version=${PREVIOUS_VERSION}"
    echo "version=${VERSION}"
    echo "target_branch=${TARGET_BRANCH}"
  } >> "${GITHUB_OUTPUT}"
fi

echo "previous_version=${PREVIOUS_VERSION}"
echo "version=${VERSION}"
