#!/usr/bin/env bash
set -euo pipefail
python3 - <<'PY'
from pathlib import Path
import yaml

auto_path = Path('.github/workflows/auto-tag-on-release-pr-merge.yml')
publish_path = Path('.github/workflows/push-gateway-helm-chart.yml')
auto_text = auto_path.read_text()
publish_text = publish_path.read_text()
# Parse first, then pin the tag producer and consumer strings whose agreement
# makes this a reachable lane rather than an orphan publisher.
yaml.safe_load(auto_text)
yaml.safe_load(publish_text)
for needle in (
    'push-chart-release/*)',
    'VERSION="${BRANCH#push-chart-release/}"',
    'TAG_PREFIX="push-chart-v"',
    '- name: Create and push tag',
    'TAG: ${{ steps.release.outputs.tag }}',
    'refs/tags/$TAG',
    '-f sha="$TARGET_SHA"',
):
    assert needle in auto_text, f'missing auto-tag gateway chart contract: {needle}'
for needle in (
    'tags: ["push-chart-v[0-9]*"]',
    'version="${INPUT_VERSION:-${REF_NAME#push-chart-v}}"',
    'refs/tags/push-chart-v${version}^{commit}',
    'deploy/charts/buzz-push-gateway',
):
    assert needle in publish_text, f'missing gateway chart publisher contract: {needle}'
PY
