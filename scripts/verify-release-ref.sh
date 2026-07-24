#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <tag-prefix> <version>" >&2
  exit 2
fi

tag_prefix=$1
version=$2

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "::error::Invalid release version '$version'" >&2
  exit 1
fi

tag="${tag_prefix}${version}"
expected_ref="refs/tags/${tag}"
if [ "${GITHUB_REF:-}" != "$expected_ref" ]; then
  echo "::error::Release must run at ${expected_ref}; got ${GITHUB_REF:-<unset>}" >&2
  exit 1
fi

head_sha=$(git rev-parse 'HEAD^{commit}')
if ! tag_sha=$(git rev-parse "refs/tags/${tag}^{commit}" 2>/dev/null); then
  echo "::error::Release tag '${tag}' is missing from the checkout" >&2
  exit 1
fi

if [ "$head_sha" != "$tag_sha" ]; then
  echo "::error::HEAD ${head_sha} does not match ${tag} commit ${tag_sha}" >&2
  exit 1
fi

echo "Verified ${expected_ref} at ${head_sha}"
