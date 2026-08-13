#!/usr/bin/env bash
set -euo pipefail
python3 - <<'PY'
from pathlib import Path

auto_path = Path('.github/workflows/auto-tag-on-release-pr-merge.yml')
publish_path = Path('.github/workflows/push-gateway-helm-chart.yml')
auto_text = auto_path.read_text()
publish_text = publish_path.read_text()
# Pin the cross-workflow strings whose agreement makes this a reachable lane
# rather than an orphan publisher. Keep this check dependency-free: GitHub
# validates workflow YAML, while this script validates the release wiring.
for needle in (
    'push-chart-release/*)',
    'VERSION="${BRANCH#push-chart-release/}"',
    'TAG_PREFIX="push-chart-v"',
    'actions/create-github-app-token@',
    'permission-contents: write',
    'GH_TOKEN: ${{ steps.release-tagger.outputs.token }}',
    'git/refs',
):
    assert needle in auto_text, f'missing auto-tag gateway chart contract: {needle}'
assert 'gh workflow run' not in auto_text, 'auto-tag must publish through the App-created tag push'
for needle in (
    'tags: ["push-chart-v[0-9]*"]',
    'version="${INPUT_VERSION:-${REF_NAME#push-chart-v}}"',
    'refs/tags/push-chart-v${version}^{commit}',
    'deploy/charts/buzz-push-gateway',
):
    assert needle in publish_text, f'missing gateway chart publisher contract: {needle}'
PY
